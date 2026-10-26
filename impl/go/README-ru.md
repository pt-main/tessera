# Tessera: реализация на Go

Чистая Go-реализация парсера, сериализатора и декодера Tessera в Go-структуры.
Без cgo, без внешних зависимостей. На синтетическом бенчмарке - порядка 80 МБ/с.

Поддерживаемые платформы: любые, под которые собирается Go-тулчейн.
Из рантайма используются только `encoding/base64`, `strconv`, `reflect` и `sort`.

> Это не замена Rust-библиотеке `tessera`. Публичный API - в Go-стиле
> (`Parse`, `ToTessera`, `Decode`), а ошибки - плоский `[]ParseError`
> либо `[]error`, а не подробный enum `ParseError`.

## Статус

- Язык: Go 1.21.
- Полнота:
  - **Парсер** для `i`, `f`, `s`, `b`, `l`, вложенных таблиц, комментариев
    и авто-закрытия блоков.
  - **Сериализация** обратно в Tessera через `ToTessera` / `TryToTessera`.
  - *(уникалная функция)* **Декодер** `Config → Go struct` с тегами, `Unmarshaler`-интерфейсом
    и опциями `ErrorMissing` / `ErrorUnused` / `CaseInsensitive`.
  - Преобразование в JSON не поддерживается.
- Хранение на map'ах повторяет спецификацию: шесть типовых map'ов на
  `Config` плюс `Inners`. Map'ы создаются лениво.

## Запуск

Бенчмарки используют стандартный testing-фреймворк Go.

```bash
go test -bench=. -benchmem
```

Доступные бенчмарки:

- `BenchmarkFlatInts` - 200 000 плоских полей типа `int`.
- `BenchmarkMixed` - 100 000 групп из `i` + `s` + `l`.
- `BenchmarkNested` - 20 000 вложенных блоков с внутренним блоком.
- `BenchmarkEscaped` - 100 000 строк в кавычках с escape-последовательностями.

Пример:

```bash
go test -bench=. -benchmem -benchtime=3x
```

`-benchtime=Nx` прогоняет каждый бенчмарк ровно `N` итераций - удобно,
когда нужны стабильные MiB/s, а не усреднение по времени.

Тесты декодера запускаются обычным `go test .` и не требуют бенчмарков.

## API

### Парсинг и генерация

```go
cfg, errs := tessera.Parse(src)              // -> *Config, []ParseError

out   := tessera.ToTessera(cfg)              // Config -> Tessera (best effort)
out2, err := tessera.TryToTessera(cfg)       // Config -> Tessera (строго)
```

`ToTessera` молча отбрасывает поля, которые нельзя записать в одну строку
(пустые byte-значения, ключи с пробелами, ключи, начинающиеся с `<` или `#`).
Если нужно это отслеживать - используйте `TryToTessera`.

### Декодирование в структуру

```go
type Tls struct {
    Enabled bool   `tessera:"enabled"`
    Cert    []byte `tessera:"cert"`
}

type Server struct {
    Host  string `tessera:"host"`
    Port  uint16 `tessera:"port"`
    Tls   Tls    `tessera:"tls"`
    Debug *bool  `tessera:"debug,optional"`
}

var s Server
if errs := tessera.Decode(cfg, &s); len(errs) != 0 {
    // errs - список ошибок, декодирование продолжается после каждой
}
```

Для тонкой настройки используется `Decoder`:

```go
d := &tessera.Decoder{
    TagName:         "tessera", // по умолчанию "tessera"
    ErrorMissing:    true,      // ошибка, если в Config нет ключа для поля
    ErrorUnused:     true,      // ошибка, если в Config есть неиспользованный ключ
    CaseInsensitive: false,     // игнорировать регистр при сопоставлении ключей
}
errs := d.Decode(cfg, &s)
```

`Decode` - это просто `(&Decoder{}).Decode(cfg, dst)` с настройками по
умолчанию. Все ошибки собираются в `[]error`, декодер не падает на первой.

#### Теги

| Тег | Значение |
|-----|----------|
| `tessera:"key"` | Имя ключа в Config. |
| `tessera:"-"` | Пропустить поле. |
| `tessera:"key,optional"` | Не требовать ключ при `ErrorMissing`. |

Если тег отсутствует, используется имя поля Go как есть
(`HTTPPort` → `"HTTPPort"`, не `"httpport"`). Tessera регистрозависима,
поэтому для экспортируемых полей явный тег почти всегда нужен.
Альтернатива - `CaseInsensitive: true`.

#### Поддерживаемые типы

| Go-тип | Источник в Config | Примечания |
|---|---|---|
| `int8..int64`, `uint8..uint64` | `Ints` | Проверка переполнения через `reflect.Overflow*`. |
| `float32`, `float64` | `Floats` | Для `float32` проверяется диапазон. |
| `string` | `Strs` | Копия строки. |
| `bool` | `Logicals` | |
| `[]byte` | `Bytes` | Копия среза. |
| `struct` | `Inners` | Рекурсивно, поле в поле. |
| `*T` | как `T` | `nil` → `reflect.New`, затем рекурсия. |
| `Option`-аналога нет; используйте `*T` | | `nil` = ключа нет. |

#### Кастомные типы: `Unmarshaler`

```go
type Duration time.Duration

func (d *Duration) UnmarshalTessera(cfg *tessera.Config, key string) error {
    n, ok := cfg.Ints[key]
    if !ok {
        return fmt.Errorf("want i for %q", key)
    }
    *d = Duration(time.Duration(n) * time.Millisecond)
    return nil
}
```

`Unmarshaler` проверяется на указателе `*T` и на адресуемом значении `T`.
Если поле - указатель, декодер аллоцирует его до вызова метода.

#### Что НЕ поддерживается

По спеке Tessera:

- `[]T` для любого `T`, кроме `byte`;
- `[N]T`;
- `interface{}`.

Такие поля дают ошибку, но не прерывают декодирование остальных полей.

#### Один ключ - несколько типов

Tessera разрешает одному ключу одновременно жить в разных типовых
map'ах. Декодер разводит значения по типу Go-поля:

```go
type Thing struct {
    X int64  `tessera:"x"` // из cfg.Ints["x"]
    Y string `tessera:"x"` // из cfg.Strs["x"]
}
```

Это работает, пока поля имеют разные Go-типы, соответствующие разным
типовым map'ам.

## Ограничения

- **Параллельного парсера нет.** Один вызов `Parse` - одна goroutine.
  Разбиение документа по нескольким goroutine - задача вызывающего кода,
  границы блоков он должен расставлять сам. (C-версия умеет в
  многопоточность, Go - нет.)
- **`i` = `int64`.** Значения вне диапазона дают `InvalidInt` и
  пропускаются, как и в Rust-референсе.
- **Строки с escape копируются.** В Rust есть zero-copy путь через
  `unescape_zero_copy`, в Go его нет.
- **Ошибки парсера плоские.** Содержат номер строки и `ErrKind`, но не
  само значение и не контекст источника. Аналога `ParseError::trace` из
  Rust здесь нет.
- **`float64` печатается в кратчайшем round-trip виде**
  (`strconv.FormatFloat(..., 'f', -1, 64)`), что совпадает с Rust `Display`
  почти всегда; субнормали и `-0` могут отличаться в крайних случаях.
- **Декодер не различает** «ключа нет» и «значение zero value» - оба
  дают `X == 0` для `int64`. Чтобы отличить, используйте `*int64`.
  Исключение - `ErrorMissing: true`: тогда отсутствие ключа даёт ошибку,
  а значение 0 проходит.
- **`Decoder` безопасен для конкурентного использования** при условии,
  что его поля не мутируются после первого вызова. Не меняйте
  `ErrorMissing` / `ErrorUnused` / `CaseInsensitive` между `Decode` на
  одной и той же копии.
- **Encoder (`struct` → `Config`) не реализован.** Для round-trip
  используйте `ToTessera` напрямую на `*Config` либо вручную заполняйте
  `Config` из структуры.

## Лицензия

Apache-2.0, как и весь проект.

См. также [`docs/SPEC.md`](../../docs/SPEC.md) и [`docs/SPEC-ru.md`](../../docs/SPEC-ru.md).