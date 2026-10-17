# Tessera C implementation

Экспериментальная standalone-реализация парсера Tessera на C.
Ориентирована на скорость (пик 4.7 ГБ/с на Intel(R) Core(TM) i7-4770HQ (8) @ 3.40 GHz): mmap, многопоточность, AVX2, арена-аллокатор, компактные 24-байтные записи. Код пригоден чтобы просто скопировать и использовать.

Поддерживаемые платформы: Linux, macOS, FreeBSD/NetBSD/OpenBSD/DragonFly, Windows (x64). Один исходный файл, без внешних зависимостей. На POSIX производительность идентична исходной версии: все горячие макросы раскрываются в те же `__builtin_*`.

> Это не замена Rust-библиотеке `tessera`. Здесь нет публичного API, генерации Tessera/JSON и подробной диагностики ошибок.

## Статус

- Язык: C11.
- Платформы: Linux, macOS, BSD, Windows (x64).
- Полнота: поддерживаются основные конструкции Tessера (`i`, `f`, `s`, `b`, `l`, вложенные таблицы, комментарии, авто-закрытие блоков в пределах чанка). Детали ошибок и полное слияние всех вложенных чанков могут отличаться от Rust-реализации.

## Сборка

### Linux / macOS / BSD (GCC или Clang)

```bash
cc -O3 -march=haswell -mtune=haswell -flto -funroll-loops \
   -fomit-frame-pointer -fno-plt -pthread -o tess tessera.c
```

Если у вас другой CPU, замените `-march=haswell` на `-march=native` или уберите AVX2-флаги:

```bash
cc -O3 -march=native -flto -funroll-loops \
   -fomit-frame-pointer -fno-plt -pthread -o tess tessera.c
```

На ARM (включая Apple Silicon) AVX2 недоступен, парсер автоматически использует скалярный путь. Достаточно убрать `-march=haswell`:

```bash
cc -O3 -flto -funroll-loops -fomit-frame-pointer -pthread -o tess tessera.c
```

### Windows

MinGW-w64 (x64):

```bash
cc -O3 -march=haswell -mtune=haswell -flto -funroll-loops \
   -fomit-frame-pointer -o tess.exe tessera.c
```

MSVC (требует VS 2022 17.3+ для C11 atomics):

```cmd
cl /O2 /arch:AVX2 /fp:precise /std:c17 /GL tessera.c
```

## Запуск

Базово, скомпилированный бинарник запускает бенчмарк по файлу.

```bash
./tess <file> [iterations] [threads]
```

- `<file>` файл в формате Tessera.
- `[iterations]` число прогонов, по умолчанию `3`.
- `[threads]` число потоков; по умолчанию определяется по физическим ядрам.

Пример:

```bash
./tess bench.tess 10 8
```

Вывод идёт в `stderr`: размер файла, число потоков, платформа, наличие AVX2, время pre-scan, время и MiB/s по итерациям, лучший результат.

## Опции сборки

- `-DTESSERA_MERGE_MASTER=0` не собирать общий master-массив для корневых записей.

## Платформы

| Подсистема    | POSIX                                      | Windows                                              |
|---------------|--------------------------------------------|------------------------------------------------------|
| File mapping  | mmap/madvise                               | CreateFileMapping/MapViewOfFile                      |
| Threading     | pthread                                    | SRWLOCK + CONDITION_VARIABLE + CreateThread          |
| Barrier       | pthread_mutex/pthread_cond                 | SRWLOCK + CONDITION_VARIABLE                        |
| Таймер        | clock_gettime(CLOCK_MONOTONIC)             | QueryPerformanceCounter                              |
| CPU count     | /proc/cpuinfo или sysctl                   | GetLogicalProcessorInformationEx                    |
| Aligned alloc | posix_memalign + MADV_HUGEPAGE             | _aligned_malloc                                      |

## Ограничения

- Нет сериализации в Tessera и JSON, нет `from_json`.
- Нет развёрнутого списка ошибок с номерами строк, как в Rust-версии.
- Параллельный парсер рассчитан на производительность; для сложных вложенных структур проверяйте корректность на своих данных.
- `-march=haswell` требует AVX2; на старых CPU или ARM используйте `-march=native` или уберите флаг, парсер автоматически переключится на скалярный путь.
- На Windows large pages не используются (требует SeLockMemoryPrivilege).

## Лицензия

Apache-2.0, как и весь проект.

См. также [`docs/SPEC.md`](../../docs/SPEC.md) и [`docs/SPEC-ru.md`](../../docs/SPEC-ru.md).