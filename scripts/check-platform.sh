#!/bin/bash
# Type-checks Null for Linux or Windows on a Mac, without a VM: `cargo check` for that
# target, so the compiler sees every path of its own (`cfg(target_os = "linux")`,
# `cfg(windows)`, `cfg(unix)`). Nothing is linked, so the C parts of dependencies
# (tree-sitter, ring) compile against small stand-in headers, and the system libraries
# gpui looks for on Linux (FreeType, xkbcommon…) are answered by stand-in pkg-config
# files: all of them written under target/, never used to build anything that runs.
#
#   scripts/check-platform.sh linux|windows [--all-targets]
#   (needs: rustup target add x86_64-unknown-linux-gnu / x86_64-pc-windows-msvc)
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
platform="${1:-}"
shift || true
case "$platform" in
    linux) target=x86_64-unknown-linux-gnu ;;
    windows) target=x86_64-pc-windows-msvc ;;
    *) echo "which: scripts/check-platform.sh linux|windows [--all-targets]" >&2; exit 1 ;;
esac
rustup target list --installed | grep -qx "$target" || {
    echo "first: rustup target add $target" >&2
    exit 1
}
stubs="$repo/target/platform-check/include"
pcs="$repo/target/platform-check/pkgconfig"
mkdir -p "$stubs" "$pcs"

header() { printf '#pragma once\n/* A stand-in for type-checking only: never linked. */\n%s\n' "$2" > "$stubs/$1"; }
header assert.h '#define assert(x) ((void)0)
#ifndef __cplusplus
#define static_assert _Static_assert
#endif'
header string.h '#include <stddef.h>
void *memcpy(void *, const void *, size_t); void *memmove(void *, const void *, size_t);
void *memset(void *, int, size_t); int memcmp(const void *, const void *, size_t);
size_t strlen(const char *); int strcmp(const char *, const char *);
int strncmp(const char *, const char *, size_t); char *strchr(const char *, int);
char *strdup(const char *); char *strncpy(char *, const char *, size_t); char *strcpy(char *, const char *);
void *memchr(const void *, int, size_t); char *strerror(int);'
header stdlib.h '#include <stddef.h>
void abort(void); void *malloc(size_t); void free(void *); void *calloc(size_t, size_t);
void *realloc(void *, size_t); long strtol(const char *, char **, int);
unsigned long strtoul(const char *, char **, int); double strtod(const char *, char **);
int abs(int); void exit(int); void qsort(void *, size_t, size_t, int (*)(const void *, const void *));'
header stdio.h '#include <stddef.h>
#include <stdarg.h>
typedef struct _FILE FILE; extern FILE *stdin, *stdout, *stderr;
int fprintf(FILE *, const char *, ...); int printf(const char *, ...);
int snprintf(char *, size_t, const char *, ...); int sprintf(char *, const char *, ...);
int vsnprintf(char *, size_t, const char *, va_list); int vfprintf(FILE *, const char *, va_list);
int fputs(const char *, FILE *); int fputc(int, FILE *); int putc(int, FILE *); int fclose(FILE *);
FILE *fopen(const char *, const char *); FILE *fdopen(int, const char *);
size_t fwrite(const void *, size_t, size_t, FILE *); size_t fread(void *, size_t, size_t, FILE *);
int fflush(FILE *);
#define EOF (-1)'
header ctype.h 'int isalpha(int); int isdigit(int); int isspace(int); int isalnum(int); int isupper(int);
int islower(int); int isxdigit(int); int isprint(int); int ispunct(int); int tolower(int); int toupper(int);'
header wctype.h '#include <stddef.h>
typedef unsigned int wint_t;
int iswspace(wint_t); int iswalpha(wint_t); int iswalnum(wint_t); int iswdigit(wint_t);
int iswupper(wint_t); int iswlower(wint_t); int iswpunct(wint_t); int iswxdigit(wint_t);
wint_t towlower(wint_t); wint_t towupper(wint_t);'
header wchar.h '#include <wctype.h>
typedef struct { int state; } mbstate_t;
size_t mbrtowc(wchar_t *, const char *, size_t, mbstate_t *); size_t wcslen(const wchar_t *);'
header time.h 'typedef long time_t; typedef long clock_t; typedef int clockid_t;
struct timespec { time_t tv_sec; long tv_nsec; };
#define CLOCK_MONOTONIC 1
#define CLOCKS_PER_SEC 1000000
int clock_gettime(clockid_t, struct timespec *); clock_t clock(void);'
header unistd.h '#include <stddef.h>
typedef long ssize_t; int close(int); ssize_t write(int, const void *, size_t);
ssize_t read(int, void *, size_t); int dup(int); long lseek(int, long, int);'
header inttypes.h '#include <stdint.h>
#define PRId32 "d"
#define PRIu32 "u"
#define PRIx32 "x"
#define PRId64 "ld"
#define PRIu64 "lu"
#define PRIx64 "lx"'
header endian.h '#define le16toh(x) (x)
#define be16toh(x) __builtin_bswap16(x)
#define htole16(x) (x)
#define htobe16(x) __builtin_bswap16(x)
#define le32toh(x) (x)
#define be32toh(x) __builtin_bswap32(x)'
mkdir -p "$stubs/sys"
header sys/types.h '#include <stddef.h>
typedef long off_t; typedef long ssize_t; typedef int pid_t; typedef unsigned int mode_t;'
header fcntl.h '#define O_RDONLY 0
#define O_WRONLY 1
#define O_RDWR 2
#define O_CREAT 64
#define O_TRUNC 512
#define O_APPEND 1024
int open(const char *, int, ...);'
header errno.h 'extern int errno;
#define ENOENT 2
#define EINTR 4
#define EAGAIN 11
#define EWOULDBLOCK EAGAIN'
header zlib.h '#define ZLIB_VERSION "1.3"'

for lib in freetype2:26.1.20 fontconfig:2.15.0 xkbcommon:1.7.0 xkbcommon-x11:1.7.0 x11:1.8.7 \
    x11-xcb:1.8.7 xcb:1.16 wayland-client:1.22.0 alsa:1.2.10 libudev:255 dbus-1:1.14.10 \
    openssl:3.0.13 zlib:1.3 libgit2:1.9.7 libssh2:1.11.0; do
    name="${lib%%:*}"
    printf 'Name: %s\nDescription: a stand-in for type-checking only\nVersion: %s\nLibs: -l%s\nCflags:\n' \
        "$name" "${lib##*:}" "$name" > "$pcs/$name.pc"
done

# Windows: its own headers over those (MSVC's names), and lib.exe's way of making an
# archive done by ar.
win="$repo/target/platform-check/windows"
mkdir -p "$win"
whead() { printf '#pragma once\n/* A stand-in for type-checking only: never linked. */\n%s\n' "$2" > "$win/$1"; }
whead intrin.h '#define _byteswap_ushort(x) __builtin_bswap16(x)
#define _byteswap_ulong(x) __builtin_bswap32(x)
#define _byteswap_uint64(x) __builtin_bswap64(x)'
whead stdlib.h "#include \"$stubs/stdlib.h\"
#include <intrin.h>
size_t wcstombs(char *, const wchar_t *, size_t);"
whead stdio.h "#include \"$stubs/stdio.h\"
FILE *_fdopen(int, const char *);"
whead io.h '#include <stdint.h>
#include <stddef.h>
intptr_t _get_osfhandle(int); int _open_osfhandle(intptr_t, int);
int _open(const char *, int, ...); int _wopen(const wchar_t *, int, ...); int _close(int);
int _read(int, void *, unsigned int); int _write(int, const void *, unsigned int);
long long _lseeki64(int, long long, int); long _lseek(int, long, int);
#define open _open
#define close _close
#define read _read
#define write _write
#define lseek _lseek'
mkdir -p "$win/sys"
whead sys/stat.h '#define _S_IREAD 0400
#define _S_IWRITE 0200
struct stat { long st_size; unsigned short st_mode; };
int stat(const char *, struct stat *); int fstat(int, struct stat *);'
whead windows.h '#include <stdint.h>
typedef void *PVOID; typedef void *HANDLE; typedef int BOOL; typedef unsigned long DWORD;
typedef union { struct { DWORD LowPart; long HighPart; }; long long QuadPart; } LARGE_INTEGER;
#define FALSE 0
#define TRUE 1
#define DUPLICATE_SAME_ACCESS 2
PVOID GetCurrentFiber(void); HANDLE GetCurrentProcess(void);
BOOL DuplicateHandle(HANDLE, HANDLE, HANDLE, HANDLE *, DWORD, BOOL, DWORD);
BOOL QueryPerformanceCounter(LARGE_INTEGER *); BOOL QueryPerformanceFrequency(LARGE_INTEGER *);
long InterlockedIncrement(long volatile *); long InterlockedDecrement(long volatile *);'
cat > "$win/lib.sh" <<'LIB'
#!/bin/bash
out=""; objects=()
for a in "$@"; do
    case "$a" in
        -out:*|/out:*) out="${a#*:}" ;;
        -nologo|/nologo) ;;
        *) objects+=("$a") ;;
    esac
done
exec ar crs "$out" "${objects[@]}"
LIB
chmod +x "$win/lib.sh"

cd "$repo"
if [ "$platform" = linux ]; then
    CC_x86_64_unknown_linux_gnu=clang AR_x86_64_unknown_linux_gnu=ar \
        CFLAGS_x86_64_unknown_linux_gnu="-isystem $stubs" \
        PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$pcs" PKG_CONFIG_LIBDIR="$pcs" \
        LIBGIT2_NO_VENDOR=1 LIBSSH2_SYS_USE_PKG_CONFIG=1 \
        cargo check --target "$target" "$@"
else
    CC_x86_64_pc_windows_msvc=clang AR_x86_64_pc_windows_msvc="$win/lib.sh" \
        CFLAGS_x86_64_pc_windows_msvc="-isystem $win -isystem $stubs" \
        PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$pcs" PKG_CONFIG_LIBDIR="$pcs" \
        LIBGIT2_NO_VENDOR=1 LIBSSH2_SYS_USE_PKG_CONFIG=1 \
        cargo check --target "$target" "$@"
fi
