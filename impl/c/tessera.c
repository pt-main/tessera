/*
 * tessera.c - parser for the Tessera data format.
 *
 * Build:
 *   cc -O3 -march=haswell -mtune=haswell -flto -funroll-loops \
 *      -fomit-frame-pointer -fno-plt -pthread -o tessera tessera.c
 *
 * Define -DTESSERA_MERGE_MASTER=0 to skip the merged master array.
 *
 * Usage:
 *   ./tessera <file> [iterations] [threads]
 */

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>
#include <pthread.h>
#include <stdatomic.h>

#ifdef __AVX2__
#include <immintrin.h>
#define HAVE_AVX2 1
#else
#define HAVE_AVX2 0
#endif

#ifndef TESSERA_MERGE_MASTER
#define TESSERA_MERGE_MASTER 1
#endif

#define ALWAYS_INLINE __attribute__((always_inline)) inline

/* ================================================================= */
/*  Byte helpers                                                     */
/* ================================================================= */

static inline const char *trim_l(const char *p, const char *e) {
    while (p < e && (unsigned char)*p <= ' ') p++;
    return p;
}
static inline const char *trim_r(const char *p, const char *e) {
    while (e > p && (unsigned char)e[-1] <= ' ') e--;
    return e;
}
static inline const char *find_nl(const char *p, const char *e) {
    const char *r = (const char *)__builtin_memchr(p, '\n', (size_t)(e - p));
    return r ? r : e;
}

/* ================================================================= */
/*  Chunked arena                                                    */
/* ================================================================= */

#define ARENA_MAX_CHUNK (64u << 20)

typedef struct ArenaChunk {
    struct ArenaChunk *next;
    size_t             used;
    size_t             cap;
    char               data[];
} ArenaChunk;

typedef struct {
    ArenaChunk *head;
    ArenaChunk *tail;
    ArenaChunk *current;
} Arena;

static void arena_init(Arena *a, size_t initial) {
    if (initial < (1u << 16)) initial = 1u << 16;
    if (initial > ARENA_MAX_CHUNK) initial = ARENA_MAX_CHUNK;
    ArenaChunk *c = (ArenaChunk *)malloc(sizeof(ArenaChunk) + initial);
    c->next    = NULL;
    c->used    = 0;
    c->cap     = initial;
    a->head    = c;
    a->tail    = c;
    a->current = c;
}

static void arena_grow(Arena *a, size_t need) {
    size_t sz = a->tail->cap * 2;
    if (sz < (1u << 16)) sz = 1u << 16;
    if (sz > ARENA_MAX_CHUNK) sz = ARENA_MAX_CHUNK;
    if (need > sz) sz = need;
    ArenaChunk *c = (ArenaChunk *)malloc(sizeof(ArenaChunk) + sz);
    c->next       = NULL;
    c->used       = 0;
    c->cap        = sz;
    a->tail->next = c;
    a->tail       = c;
}

static inline void *arena_alloc(Arena *a, size_t n) {
    n = (n + 15) & ~(size_t)15;
    ArenaChunk *c = a->current;
    while (c) {
        if (c->used + n <= c->cap) break;
        c = c->next;
    }
    if (__builtin_expect(!c, 0)) {
        arena_grow(a, n);
        c = a->tail;
    }
    void *p = c->data + c->used;
    c->used += n;
    a->current = c;
    return p;
}

static void arena_reset(Arena *a) {
    for (ArenaChunk *c = a->head; c; c = c->next) c->used = 0;
    a->current = a->head;
}

static void arena_destroy(Arena *a) {
    ArenaChunk *c = a->head;
    while (c) {
        ArenaChunk *n = c->next;
        free(c);
        c = n;
    }
    a->head = a->tail = a->current = NULL;
}

/* ================================================================= */
/*  Data model - 24-byte Entry                                       */
/*                                                                   */
/*  key  : 8 bytes  pointer into the mmap'd file (or into a stack    */
/*                  buffer for the synthetic "<name>" case)          */
/*  v    : 8 bytes  payload (int64 / double / pointer / Table*)      */
/*  meta : 8 bytes  packed (key_len:24, s_len:24, type:8, flags:8)   */
/*                                                                   */
/*  25 % less entry-store traffic than the old 32-byte layout - the  */
/*  dominant cost once the parse loop itself is memory-bound.        */
/* ================================================================= */

typedef enum {
    T_INT   = 1,
    T_STR   = 2,
    T_FLOAT = 3,
    T_BYTES = 4,
    T_BOOL  = 5,
    T_INNER = 6,
} VType;

typedef struct Table Table;

typedef struct {
    const char *key;
    union {
        int64_t     i;
        double      f;
        uint64_t    bits;
        const char *ptr;
        Table      *inner;
    } v;
    uint64_t    meta;
} Entry;

_Static_assert(sizeof(Entry) == 24, "Entry must be 24 bytes");

#define E_META_LEN_BITS  24
#define E_META_LEN_MAX   ((1u << E_META_LEN_BITS) - 1u)

#define E_SET(e, kl, sl, ty)                                              \
    ((e)->meta = (uint64_t)(uint32_t)(kl)                                 \
               | ((uint64_t)(uint32_t)(sl) << 24)                         \
               | ((uint64_t)(uint8_t) (ty) << 48))

#define E_KEYLEN(e) ((uint32_t)( (e)->meta        & 0xFFFFFFu))
#define E_SLEN(e)   ((uint32_t)(((e)->meta >> 24) & 0xFFFFFFu))
#define E_TYPE(e)   ((uint8_t) (((e)->meta >> 48) & 0xFFu))

struct Table {
    Entry   *entries;
    uint32_t count;
    uint32_t cap;
};

/* ================================================================= */
/*  Parser state                                                     */
/* ================================================================= */

typedef struct { Table *table; const char *name; uint32_t name_len; } StackEntry;

typedef struct {
    Table      *root;
    Entry      *root_owned;
    Arena       arena;
    StackEntry *stack;
    size_t      stack_size;
    size_t      stack_cap;
    int         errors;
    int         root_fixed;

    uint32_t    start_depth;
    uint32_t    virtual_depth;
    int         outside_range;
} Parser;

static void parse_chunk_slow(Parser *P, const char *data, size_t size);

/* ---- Table ops ---- */

static void table_prealloc(Table *t, uint32_t cap) {
    if (cap < 8) cap = 8;
    t->entries = (Entry *)malloc((size_t)cap * sizeof(Entry));
    t->cap     = cap;
    t->count   = 0;
}

static ALWAYS_INLINE Entry *table_append(Parser *P, Table *t) {
    if (__builtin_expect(t->count == t->cap, 0)) {
        if (t == P->root && P->root_fixed) {
            P->errors++;
            return &t->entries[0];
        }
        uint32_t nc = t->cap ? t->cap * 2 : 8;
        if (t == P->root) {
            t->entries = (Entry *)realloc(t->entries,
                                          (size_t)nc * sizeof(Entry));
            P->root_owned = t->entries;
        } else {
            Entry *ne = (Entry *)arena_alloc(&P->arena,
                                             (size_t)nc * sizeof(Entry));
            if (t->count) memcpy(ne, t->entries,
                                 (size_t)t->count * sizeof(Entry));
            t->entries = ne;
        }
        t->cap = nc;
    }
    return &t->entries[t->count++];
}

/* ---- Stack ops ---- */

static ALWAYS_INLINE void stack_push(Parser *P, Table *t,
                                     const char *nm, uint32_t nl) {
    if (__builtin_expect(P->stack_size == P->stack_cap, 0)) {
        P->stack_cap = P->stack_cap ? P->stack_cap * 2 : 64;
        P->stack = (StackEntry *)realloc(P->stack,
                                         P->stack_cap * sizeof(StackEntry));
    }
    P->stack[P->stack_size++] = (StackEntry){ t, nm, nl };
}

static inline void pop_and_insert(Parser *P) {
    StackEntry top    = P->stack[--P->stack_size];
    Table     *parent = P->stack[P->stack_size - 1].table;
    Entry     *e      = table_append(P, parent);
    e->key     = top.name;
    e->v.inner = top.table;
    E_SET(e, top.name_len, 0, T_INNER);
}

/* ================================================================= */
/*  Scalar parsers                                                   */
/* ================================================================= */

static ALWAYS_INLINE bool parse_i64(const char *p, size_t n, int64_t *out) {
    if (!n) return false;
    bool neg = false;
    if (*p == '-')      { neg = true; p++; n--; if (!n) return false; }
    else if (*p == '+') { p++; n--; if (!n) return false; }
    uint64_t v = 0;
    for (size_t i = 0; i < n; i++) {
        unsigned c = (unsigned)((unsigned char)p[i] - '0');
        if (c > 9) return false;
        v = v * 10 + c;
    }
    *out = neg ? (int64_t)(0 - v) : (int64_t)v;
    return true;
}

static bool parse_f64(const char *p, size_t n, double *out) {
    if (!n) return false;
    size_t i = 0;
    bool neg = false;
    if (p[i] == '-')      { neg = true; i++; }
    else if (p[i] == '+') { i++; }
    if (i >= n) goto fallback;
    uint64_t ip = 0;
    bool any = false;
    while (i < n && (unsigned)((unsigned char)p[i] - '0') <= 9) {
        ip = ip * 10 + (uint64_t)(p[i] - '0');
        i++; any = true;
    }
    if (i < n && p[i] == '.') {
        i++;
        uint64_t fp = 0;
        double   scale = 1.0;
        while (i < n && (unsigned)((unsigned char)p[i] - '0') <= 9) {
            fp = fp * 10 + (uint64_t)(p[i] - '0');
            scale *= 10.0;
            i++; any = true;
        }
        if (i != n) goto fallback;
        double v = (double)ip + (double)fp / scale;
        *out = neg ? -v : v;
        return any;
    }
    if (i != n) goto fallback;
    *out = neg ? -(double)ip : (double)ip;
    return any;
fallback:
    {
        char buf[64];
        if (n < sizeof buf) {
            memcpy(buf, p, n); buf[n] = 0;
            char *endp; *out = strtod(buf, &endp);
            return endp == buf + n;
        }
        char *tmp = (char *)malloc(n + 1);
        memcpy(tmp, p, n); tmp[n] = 0;
        char *endp; *out = strtod(tmp, &endp);
        bool ok = (endp == tmp + n); free(tmp); return ok;
    }
}

static ALWAYS_INLINE bool parse_bool(const char *p, size_t n, bool *out) {
    if (n == 4 && memcmp(p, "true",  4) == 0) { *out = true;  return true; }
    if (n == 5 && memcmp(p, "false", 5) == 0) { *out = false; return true; }
    if (n == 1) {
        if (*p == '1') { *out = true;  return true; }
        if (*p == '0') { *out = false; return true; }
    }
    return false;
}

/* ================================================================= */
/*  Base64 / escape                                                  */
/* ================================================================= */

static int8_t B64[256];

static void b64_init(void) {
    memset(B64, -1, sizeof B64);
    const char *A = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
                    "abcdefghijklmnopqrstuvwxyz"
                    "0123456789+/";
    for (int i = 0; i < 64; i++) B64[(uint8_t)A[i]] = (int8_t)i;
}

static int base64_decode(const char *p, size_t n, Arena *a, const char **out) {
    while (n && p[n - 1] == '=') n--;
    if (!n) { *out = NULL; return 0; }
    size_t full = n & ~3u;
    size_t rem  = n - full;
    size_t cap  = (full >> 2) * 3 + 3;
    char  *buf  = (char *)arena_alloc(a, cap);
    size_t j    = 0;
    for (size_t i = 0; i < full; i += 4) {
        int8_t a0 = B64[(uint8_t)p[i    ]];
        int8_t a1 = B64[(uint8_t)p[i + 1]];
        int8_t a2 = B64[(uint8_t)p[i + 2]];
        int8_t a3 = B64[(uint8_t)p[i + 3]];
        if ((a0 | a1 | a2 | a3) < 0) return -1;
        buf[j++] = (char)((a0 << 2) | (a1 >> 4));
        buf[j++] = (char)(((a1 & 0xF) << 4) | (a2 >> 2));
        buf[j++] = (char)(((a2 & 0x3) << 6) | a3);
    }
    if (rem) {
        int8_t a0 = rem > 0 ? B64[(uint8_t)p[full    ]] : 0;
        int8_t a1 = rem > 1 ? B64[(uint8_t)p[full + 1]] : 0;
        int8_t a2 = rem > 2 ? B64[(uint8_t)p[full + 2]] : 0;
        if ((a0 | a1 | a2) < 0) return -1;
        buf[j++] = (char)((a0 << 2) | (a1 >> 4));
        if (rem >= 3) buf[j++] = (char)(((a1 & 0xF) << 4) | (a2 >> 2));
    }
    *out = buf;
    return (int)j;
}

static bool unescape_default(const char *p, size_t n, Arena *a,
                             const char **out, size_t *out_len) {
    char  *buf = (char *)arena_alloc(a, n + 1);
    size_t j   = 0;
    for (size_t i = 0; i < n; i++) {
        char c = p[i];
        if (c != '\\') { buf[j++] = c; continue; }
        if (++i >= n) return false;
        switch (p[i]) {
        case '\\': buf[j++] = '\\'; break;
        case '"':  buf[j++] = '"';  break;
        case 'n':  buf[j++] = '\n'; break;
        case 'r':  buf[j++] = '\r'; break;
        case 't':  buf[j++] = '\t'; break;
        case 'u': {
            if (i + 4 >= n) return false;
            uint32_t cp = 0;
            for (int k = 0; k < 4; k++) {
                char h = p[++i];
                cp <<= 4;
                if      (h >= '0' && h <= '9') cp |= (uint32_t)(h - '0');
                else if (h >= 'a' && h <= 'f') cp |= (uint32_t)(h - 'a' + 10);
                else if (h >= 'A' && h <= 'F') cp |= (uint32_t)(h - 'A' + 10);
                else return false;
            }
            if (cp < 0x80) {
                buf[j++] = (char)cp;
            } else if (cp < 0x800) {
                buf[j++] = (char)(0xC0 | (cp >> 6));
                buf[j++] = (char)(0x80 | (cp & 0x3F));
            } else {
                buf[j++] = (char)(0xE0 | (cp >> 12));
                buf[j++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                buf[j++] = (char)(0x80 | (cp & 0x3F));
            }
            break;
        }
        default: return false;
        }
    }
    buf[j] = 0;
    *out = buf;
    *out_len = j;
    return true;
}

/* ================================================================= */
/*  Field dispatch                                                   */
/* ================================================================= */

static ALWAYS_INLINE void handle_field(Parser *P, Table *cur,
                                       const char *key, uint32_t key_len,
                                       const char *vt,  uint32_t vt_len,
                                       const char *val, uint32_t val_len) {
    if (__builtin_expect(vt_len != 1, 0)) { P->errors++; return; }
    if (__builtin_expect(key_len > E_META_LEN_MAX, 0)) { P->errors++; return; }

    Entry *e;
    switch (vt[0]) {
    case 'i': {
        int64_t iv;
        if (!parse_i64(val, val_len, &iv)) { P->errors++; return; }
        e = table_append(P, cur);
        e->key = key;
        e->v.i = iv;
        E_SET(e, key_len, 0, T_INT);
        return;
    }
    case 'f': {
        double dv;
        if (!parse_f64(val, val_len, &dv)) { P->errors++; return; }
        e = table_append(P, cur);
        e->key = key;
        e->v.f = dv;
        E_SET(e, key_len, 0, T_FLOAT);
        return;
    }
    case 's': {
        const char *s  = val;
        size_t      sl = val_len;
        if (sl >= 2 && s[0] == '"' && s[sl - 1] == '"') { s++; sl -= 2; }
        const char *out;
        size_t      ol;
        if (__builtin_expect(memchr(s, '\\', sl) == NULL, 1)) {
            out = s; ol = sl;
        } else if (!unescape_default(s, sl, &P->arena, &out, &ol)) {
            P->errors++; return;
        }
        if (__builtin_expect(ol > E_META_LEN_MAX, 0)) {
            P->errors++; return;
        }
        e = table_append(P, cur);
        e->key   = key;
        e->v.ptr = out;
        E_SET(e, key_len, (uint32_t)ol, T_STR);
        return;
    }
    case 'b': {
        const char *out;
        int ol = base64_decode(val, val_len, &P->arena, &out);
        if (ol < 0) { P->errors++; return; }
        if (__builtin_expect((uint32_t)ol > E_META_LEN_MAX, 0)) {
            P->errors++; return;
        }
        e = table_append(P, cur);
        e->key   = key;
        e->v.ptr = out;
        E_SET(e, key_len, (uint32_t)ol, T_BYTES);
        return;
    }
    case 'l': {
        const char *vs = trim_l(val, val + val_len);
        const char *ve = trim_r(vs, val + val_len);
        bool bv;
        if (!parse_bool(vs, (size_t)(ve - vs), &bv)) { P->errors++; return; }
        e = table_append(P, cur);
        e->key    = key;
        e->v.bits = bv ? 1 : 0;
        E_SET(e, key_len, 0, T_BOOL);
        return;
    }
    default:
        P->errors++;
        return;
    }
}

/* ================================================================= */
/*  Slow parser                                                      */
/* ================================================================= */

static void parse_chunk_slow(Parser *P, const char *data, size_t size) {
    const char *p   = data;
    const char *end = data + size;

    while (p < end) {
        const char *nl = find_nl(p, end);
        const char *ls = trim_l(p, nl);
        const char *le = trim_r(ls, nl);
        p = nl + 1;

        if (ls == le) continue;
        char c0 = *ls;
        if (c0 == '#') continue;

        if (c0 == '<') {
            P->virtual_depth++;
            if (P->outside_range) continue;
            const char *nm   = trim_l(ls + 1, le);
            const char *nme  = trim_r(nm, le);
            uint32_t    nlen = (uint32_t)(nme - nm);
            if (!nlen) { P->errors++; continue; }
            Table *sub = (Table *)arena_alloc(&P->arena, sizeof(Table));
            sub->entries = NULL; sub->cap = 0; sub->count = 0;
            stack_push(P, sub, nm, nlen);
            continue;
        }

        if (c0 == '>') {
            if (P->virtual_depth == 0) { P->errors++; continue; }
            P->virtual_depth--;
            if (P->virtual_depth < P->start_depth) P->outside_range = 1;
            if (P->outside_range) continue;
            if (P->stack_size > 1) pop_and_insert(P);
            else P->errors++;
            continue;
        }

        if (P->outside_range) continue;

        const char *sp1 = ls;
        while (sp1 < le && *sp1 != ' ') sp1++;
        if (sp1 == le) { P->errors++; continue; }
        const char *sp2 = sp1 + 1;
        while (sp2 < le && *sp2 != ' ') sp2++;
        if (sp2 == le) { P->errors++; continue; }

        handle_field(P, P->stack[P->stack_size - 1].table,
                     ls,      (uint32_t)(sp1 - ls),
                     sp1 + 1, (uint32_t)(sp2 - sp1 - 1),
                     sp2 + 1, (uint32_t)(le - sp2 - 1));
    }
}

/* ================================================================= */
/*  AVX2 fast parser                                                 */
/* ================================================================= */

#if HAVE_AVX2
static void parse_chunk_fast(Parser *P, const char *data, size_t size) {
    const char *p   = data;
    const char *end = data + size;

    const __m256i nl_v = _mm256_set1_epi8('\n');
    const __m256i sp_v = _mm256_set1_epi8(' ');

    while (p + 64 <= end) {
        _mm_prefetch(p + 128, _MM_HINT_T0);
        _mm_prefetch(p + 192, _MM_HINT_T0);

        __m256i v0 = _mm256_loadu_si256((const __m256i *)p);
        __m256i v1 = _mm256_loadu_si256((const __m256i *)(p + 32));

        uint32_t nl0 = (uint32_t)_mm256_movemask_epi8(
                           _mm256_cmpeq_epi8(v0, nl_v));
        uint32_t nl1 = (uint32_t)_mm256_movemask_epi8(
                           _mm256_cmpeq_epi8(v1, nl_v));
        uint64_t nlm = (uint64_t)nl0 | ((uint64_t)nl1 << 32);

        if (__builtin_expect(nlm == 0, 0)) {
            const char *nl_real = (const char *)memchr(p + 64, '\n',
                                                       (size_t)(end - p - 64));
            if (!nl_real) {
                parse_chunk_slow(P, p, (size_t)(end - p));
                return;
            }
            parse_chunk_slow(P, p, (size_t)(nl_real - p + 1));
            p = nl_real + 1;
            continue;
        }

        uint32_t sp0 = (uint32_t)_mm256_movemask_epi8(
                           _mm256_cmpeq_epi8(v0, sp_v));
        uint32_t sp1 = (uint32_t)_mm256_movemask_epi8(
                           _mm256_cmpeq_epi8(v1, sp_v));
        uint64_t spm = (uint64_t)sp0 | ((uint64_t)sp1 << 32);

        uint32_t ws0 = (uint32_t)_mm256_movemask_epi8(
                           _mm256_cmpeq_epi8(_mm256_max_epu8(v0, sp_v), sp_v));
        uint32_t ws1 = (uint32_t)_mm256_movemask_epi8(
                           _mm256_cmpeq_epi8(_mm256_max_epu8(v1, sp_v), sp_v));
        uint64_t wsm = (uint64_t)ws0 | ((uint64_t)ws1 << 32);

        uint64_t nws_full = ~wsm;

        uint64_t rem        = nlm;
        uint64_t prev_upper = 0;
        int      last_nl    = 63 - __builtin_clzll(nlm);

        do {
            int nl_pos = __builtin_ctzll(rem);
            rem &= rem - 1;

            uint64_t upper     = (1ULL << nl_pos) - 1;
            uint64_t line_mask = upper & ~prev_upper;
            uint64_t line_nws  = nws_full & line_mask;

            if (__builtin_expect(line_nws != 0, 1)) {
                int  lead  = __builtin_ctzll(line_nws);
                int  trail = 63 - __builtin_clzll(line_nws);
                char c0    = p[lead];

                if (__builtin_expect(c0 == '<', 0)) {
                    P->virtual_depth++;
                    if (!P->outside_range) {
                        uint64_t after = line_nws &
                                         ~((1ULL << (lead + 1)) - 1);
                        if (__builtin_expect(after != 0, 1)) {
                            int    ns  = __builtin_ctzll(after);
                            Table *sub = (Table *)arena_alloc(&P->arena,
                                                             sizeof(Table));
                            sub->entries = NULL; sub->cap = 0; sub->count = 0;
                            stack_push(P, sub, p + ns,
                                       (uint32_t)(trail - ns + 1));
                        } else P->errors++;
                    }
                } else if (__builtin_expect(c0 == '>', 0)) {
                    if (P->virtual_depth == 0) {
                        P->errors++;
                    } else {
                        P->virtual_depth--;
                        if (P->virtual_depth < P->start_depth)
                            P->outside_range = 1;
                        if (!P->outside_range) {
                            if (trail == lead && P->stack_size > 1)
                                pop_and_insert(P);
                            else P->errors++;
                        }
                    }
                } else if (__builtin_expect(c0 == '#', 0)) {
                    /* comment */
                } else if (!P->outside_range) {
                    uint64_t sp_a = spm & line_mask & ~((1ULL << lead) - 1);
                    if (__builtin_expect(sp_a != 0, 1)) {
                        int s1 = __builtin_ctzll(sp_a);
                        uint64_t sp_b = spm & line_mask &
                                        ~((1ULL << (s1 + 1)) - 1);
                        if (__builtin_expect(sp_b != 0, 1)) {
                            int s2 = __builtin_ctzll(sp_b);
                            handle_field(P,
                                         P->stack[P->stack_size - 1].table,
                                         p + lead,   (uint32_t)(s1 - lead),
                                         p + s1 + 1, (uint32_t)(s2 - s1 - 1),
                                         p + s2 + 1, (uint32_t)(trail - s2));
                        } else P->errors++;
                    } else P->errors++;
                }
            }
            prev_upper = upper | (1ULL << nl_pos);
        } while (rem);

        p += last_nl + 1;
    }

    if (p < end) parse_chunk_slow(P, p, (size_t)(end - p));
}
#endif

static void parse_chunk(Parser *P, const char *data, size_t size) {
#if HAVE_AVX2
    parse_chunk_fast(P, data, size);
#else
    parse_chunk_slow(P, data, size);
#endif
    while (P->stack_size > 1) pop_and_insert(P);
}

/* ================================================================= */
/*  Split discovery - allows splits at any depth                     */
/* ================================================================= */

static size_t find_splits(const char *data, size_t size,
                          size_t *out_off, uint32_t *out_depth,
                          size_t want) {
    out_off[0]   = 0;
    out_depth[0] = 0;
    size_t n = 1;
    if (want <= 1 || size == 0) return n;

    size_t target      = size / want;
    if (target == 0) target = 1;
    size_t next_target = target;

    uint32_t depth = 0;
    const char *p     = data;
    const char *end   = data + size;

#if HAVE_AVX2
    const __m256i nl_v = _mm256_set1_epi8('\n');
#endif

    while (p < end && n < want) {
        const char *line_start = p;
        const char *nl         = NULL;

#if HAVE_AVX2
        while (p + 32 <= end) {
            __m256i  v = _mm256_loadu_si256((const __m256i *)p);
            uint32_t m = (uint32_t)_mm256_movemask_epi8(
                             _mm256_cmpeq_epi8(v, nl_v));
            if (m) { nl = p + __builtin_ctz(m); break; }
            p += 32;
        }
        if (!nl) {
            if (p >= end) break;
            const char *r = (const char *)memchr(p, '\n',
                                                 (size_t)(end - p));
            nl = r ? r : end;
        }
#else
        const char *r = (const char *)memchr(p, '\n', (size_t)(end - p));
        nl = r ? r : end;
#endif

        const char *q = line_start;
        while (q < nl && (unsigned char)*q <= ' ') q++;
        char c0 = (q < nl) ? *q : 0;
        if (c0 == '<') depth++;
        else if (c0 == '>') {
            const char *q2 = q + 1;
            while (q2 < nl && (unsigned char)*q2 <= ' ') q2++;
            if (q2 == nl && depth > 0) depth--;
        }

        p = nl + 1;
        size_t pos = (size_t)(p - data);

        if (pos >= next_target && p < end) {
            const char *nq = p;
            while (nq < end && (unsigned char)*nq <= ' ') nq++;
            int safe = (nq < end && *nq != '>');

            if (safe && (depth == 0 || pos >= next_target + target)) {
                out_off[n]   = pos;
                out_depth[n] = depth;
                n++;
                next_target  = pos + target;
            }
        }
    }
    return n;
}

/* ================================================================= */
/*  Barrier                                                          */
/* ================================================================= */

typedef struct {
    pthread_mutex_t mu;
    pthread_cond_t  cv;
    int             count;
    int             n;
    int             generation;
} Barrier;

static void barrier_init(Barrier *b, int n) {
    pthread_mutex_init(&b->mu, NULL);
    pthread_cond_init(&b->cv, NULL);
    b->count      = 0;
    b->n          = n;
    b->generation = 0;
}
static void barrier_wait(Barrier *b) {
    pthread_mutex_lock(&b->mu);
    int gen = b->generation;
    if (++b->count == b->n) {
        b->count = 0;
        b->generation++;
        pthread_cond_broadcast(&b->cv);
    } else {
        while (b->generation == gen) pthread_cond_wait(&b->cv, &b->mu);
    }
    pthread_mutex_unlock(&b->mu);
}

/* ================================================================= */
/*  Worker pool                                                      */
/* ================================================================= */

typedef struct Pool Pool;

typedef struct {
    int       id;
    Pool     *pool;
    pthread_t tid;
    Parser    P;
    uint32_t  table_cap;
    size_t    arena_cap;
    size_t    stack_cap;
} Worker;

struct Pool {
    Barrier    b_start;
    Barrier    b_done;
    _Atomic int shutdown;

    const char *file_data;
    size_t     *bounds;
    uint32_t   *depths;

    Worker *workers;

#if TESSERA_MERGE_MASTER
    Entry        *master;
    size_t        master_cap;
    size_t       *worker_off;
    _Atomic int   direct_ready;
#endif
};

static void *worker_main(void *arg) {
    Worker *w    = (Worker *)arg;
    Pool   *pool = w->pool;

    w->P.root = (Table *)malloc(sizeof(Table));
    table_prealloc(w->P.root, w->table_cap);
    w->P.root_owned = w->P.root->entries;

    arena_init(&w->P.arena, w->arena_cap);

    w->P.stack     = (StackEntry *)malloc(w->stack_cap * sizeof(StackEntry));
    w->P.stack_cap = w->stack_cap;

    for (;;) {
        barrier_wait(&pool->b_start);
        if (atomic_load_explicit(&pool->shutdown, memory_order_acquire)) break;

        size_t start = pool->bounds[w->id];
        size_t end   = pool->bounds[w->id + 1];

        w->P.start_depth   = pool->depths[w->id];
        w->P.virtual_depth = w->P.start_depth;
        w->P.outside_range = 0;

#if TESSERA_MERGE_MASTER
        if (w->P.start_depth == 0 &&
            atomic_load_explicit(&pool->direct_ready, memory_order_acquire)) {
            size_t lo = pool->worker_off[w->id];
            size_t hi = pool->worker_off[w->id + 1];
            w->P.root->entries = pool->master + lo;
            w->P.root->cap     = (uint32_t)(hi - lo);
            w->P.root_fixed    = 1;
        } else
#endif
        {
            w->P.root->entries = w->P.root_owned;
            w->P.root->cap     = w->table_cap;
            w->P.root_fixed    = 0;
        }

        w->P.root->count = 0;
        arena_reset(&w->P.arena);
        w->P.stack_size  = 0;
        w->P.errors      = 0;
        w->P.stack[0]    = (StackEntry){ w->P.root, "", 0 };
        w->P.stack_size  = 1;

        parse_chunk(&w->P, pool->file_data + start, end - start);

        barrier_wait(&pool->b_done);
    }

    free(w->P.root_owned);
    free(w->P.root);
    free(w->P.stack);
    arena_destroy(&w->P.arena);
    return NULL;
}

/* ================================================================= */
/*  CPU detection                                                    */
/* ================================================================= */

#if defined(__linux__)
static int physical_core_count(void) {
    FILE *f = fopen("/proc/cpuinfo", "r");
    if (!f) return 0;
    char line[256];
    int  cores = 0;
    while (fgets(line, sizeof line, f)) {
        if (sscanf(line, "cpu cores : %d", &cores) == 1) break;
    }
    fclose(f);
    return cores;
}
#else
static int physical_core_count(void) { return 0; }
#endif

/* ================================================================= */
/*  Utilities                                                        */
/* ================================================================= */

static double now_sec(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

static void *aligned_alloc_huge(size_t bytes) {
    const size_t align = 2u << 20;
    if (bytes < align) bytes = align;
    size_t rounded = (bytes + align - 1) & ~(align - 1);

    void *p = NULL;
    if (posix_memalign(&p, align, rounded) != 0) return malloc(bytes);
#if defined(MADV_HUGEPAGE)
    madvise(p, rounded, MADV_HUGEPAGE);
#endif
    return p;
}

/* ================================================================= */
/*  Main                                                             */
/* ================================================================= */

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <file> [iterations] [threads]\n", argv[0]);
        return 1;
    }
    int iters = argc > 2 ? atoi(argv[2]) : 3;
    int nthr  = argc > 3 ? atoi(argv[3]) : 0;
    if (iters < 1) iters = 1;

    b64_init();

    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }

    struct stat st;
    if (fstat(fd, &st)) { perror("fstat"); return 1; }
    size_t size = (size_t)st.st_size;

    const char *data = (const char *)mmap(NULL, size, PROT_READ,
                                          MAP_PRIVATE, fd, 0);
    if (data == MAP_FAILED) { perror("mmap"); return 1; }
    madvise((void *)data, size, MADV_WILLNEED);

    int detected = physical_core_count();
    if (nthr <= 0) {
        nthr = detected > 0 ? detected : (int)sysconf(_SC_NPROCESSORS_ONLN);
        if (nthr < 1)  nthr = 1;
        if (nthr > 32) nthr = 32;
    }
    fprintf(stderr,
            "file=%s  size=%.1f MiB  threads=%d  physical_cores=%d  "
            "avx2=%d  merge=%d  entry=%zu\n",
            argv[1], (double)size / (1024.0 * 1024.0), nthr, detected,
            HAVE_AVX2, TESSERA_MERGE_MASTER, sizeof(Entry));

    size_t   *bounds = (size_t *)malloc((size_t)(nthr + 1) * sizeof(size_t));
    uint32_t *depths = (uint32_t *)calloc((size_t)nthr, sizeof(uint32_t));
    double    t0     = now_sec();
    size_t    nsplit = find_splits(data, size, bounds, depths, (size_t)nthr);
    double    t1     = now_sec();
    fprintf(stderr, "pre-scan: %.3f ms  splits=%zu\n",
            (t1 - t0) * 1000.0, nsplit);

    for (size_t i = nsplit; i < (size_t)nthr; i++) {
        bounds[i] = size;
        depths[i] = 0;
    }
    bounds[nthr] = size;

    size_t max_chunk = 0;
    for (int i = 0; i < nthr; i++) {
        size_t c = bounds[i + 1] - bounds[i];
        if (c > max_chunk) max_chunk = c;
    }

    Pool pool = {0};
    barrier_init(&pool.b_start, nthr + 1);
    barrier_init(&pool.b_done,  nthr + 1);
    pool.bounds    = bounds;
    pool.depths    = depths;
    pool.file_data = data;
    pool.workers   = (Worker *)calloc((size_t)nthr, sizeof(Worker));
#if TESSERA_MERGE_MASTER
    pool.worker_off = (size_t *)calloc((size_t)(nthr + 1), sizeof(size_t));
#endif

    for (int i = 0; i < nthr; i++) {
        Worker *w = &pool.workers[i];
        w->id        = i;
        w->pool      = &pool;
        w->table_cap = (uint32_t)(max_chunk / 16 + 1024);
        if (w->table_cap > (1u << 26)) w->table_cap = (1u << 26);
        w->arena_cap = max_chunk / 8 + (1u << 20);
        if (w->arena_cap > (16u << 20)) w->arena_cap = 16u << 20;
        w->stack_cap = 256;
    }

    for (int i = 0; i < nthr; i++) {
        pthread_create(&pool.workers[i].tid, NULL, worker_main, &pool.workers[i]);
    }

    double best = 1e30;

    for (int it = 0; it < iters; it++) {
        double t = now_sec();

        barrier_wait(&pool.b_start);
        barrier_wait(&pool.b_done);

        size_t total   = 0;
        int    total_e = 0;
        for (int i = 0; i < nthr; i++) {
            if (pool.depths[i] == 0)
                total += pool.workers[i].P.root->count;
            total_e += pool.workers[i].P.errors;
        }

#if TESSERA_MERGE_MASTER
        if (it == 0) {
            pool.worker_off[0] = 0;
            for (int i = 0; i < nthr; i++) {
                uint32_t c = pool.depths[i] == 0
                             ? pool.workers[i].P.root->count : 0;
                size_t slice = (size_t)c + c / 4 + 64;
                pool.worker_off[i + 1] = pool.worker_off[i] + slice;
            }
            pool.master_cap = pool.worker_off[nthr];
            if (pool.master_cap == 0) pool.master_cap = 4096;
            pool.master = (Entry *)aligned_alloc_huge(
                              pool.master_cap * sizeof(Entry));

            for (int i = 0; i < nthr; i++) {
                if (pool.depths[i] != 0) continue;
                uint32_t c = pool.workers[i].P.root->count;
                if (c)
                    memcpy(pool.master + pool.worker_off[i],
                           pool.workers[i].P.root->entries,
                           (size_t)c * sizeof(Entry));
            }
            atomic_store_explicit(&pool.direct_ready, 1,
                                  memory_order_release);
        }
#endif

        double dt = now_sec() - t;
        if (dt < best) best = dt;
        fprintf(stderr,
                "iter %2d: %8.3f ms  %7.1f MiB/s  err=%d  entries=%zu\n",
                it, dt * 1000.0,
                (double)size / (1024.0 * 1024.0) / dt,
                total_e, total);
    }

    fprintf(stderr, "BEST: %.1f MiB/s  (threads=%d)\n",
            (double)size / (1024.0 * 1024.0) / best, nthr);

    atomic_store(&pool.shutdown, 1);
    barrier_wait(&pool.b_start);
    for (int i = 0; i < nthr; i++) pthread_join(pool.workers[i].tid, NULL);

#if TESSERA_MERGE_MASTER
    free(pool.master);
    free(pool.worker_off);
#endif
    free(pool.workers);
    free(bounds);
    free(depths);

    munmap((void *)data, size);
    close(fd);
    return 0;
}