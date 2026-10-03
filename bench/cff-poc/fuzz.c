/* Mutation fuzz for the #25 POC (docs/cff-rust-vs-c.md): corrupt bytes of a
   font's CFF table and run every glyph through stock stb_truetype, cff.c and
   cff.rs, each in a child process under ASan + UBSan with a time limit, and
   count how each one ends.

     fuzz <font> <mutants> <seed> [save-dir]

   A save-dir keeps the first font that ends each implementation in each bad
   way, for replay with `fuzz one <font>...`. */
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#include "cff.h"

void *stb_open(const unsigned char *data, int face);
int stb_glyph_count(const void *font);
int stb_shape(const void *font, int glyph, cff_vertex **out);
void stb_free_shape(const void *font, cff_vertex *v);
void stb_box(const void *font, int glyph, int box[4]);
int rs_run(const uint8_t *data, size_t len, int face, uint64_t *hash);

const char *__asan_default_options(void) { return "exitcode=99:detect_leaks=0"; }
const char *__ubsan_default_options(void) { return "exitcode=99"; }

/* Child exit codes. */
enum { RAN = 0, REJECTED = 1, PANICKED = 2, MEMORY = 99 };
enum { N_RAN, N_REJECTED, N_PANIC, N_MEMORY, N_ASSERT, N_CRASH, N_HANG, N_OTHER, N_KINDS };
static const char *kinds[N_KINDS] = {"ran", "rejected", "panic", "memory/UB error", "assert", "crash", "hang", "other"};
static const char *impls[3] = {"stb", "cff.c", "cff.rs"};

/* Each child leaves a hash of every glyph's outline and box here, so the
   parent can tell whether the ones that ran drew the same thing. */
static uint64_t *hashes;

/* FNV-1a; fuzz_rs.rs hashes the same bytes the same way. */
static uint64_t fnv(uint64_t h, const void *p, size_t n) {
    const uint8_t *b = p;
    for (size_t i = 0; i < n; i++) h = (h ^ b[i]) * 0x100000001b3ull;
    return h;
}

/* n is -1 for a glyph the reader refused. */
static uint64_t glyph_hash(uint64_t h, int32_t n, const int box[4], const cff_vertex *v) {
    h = fnv(h, &n, 4);
    h = fnv(h, box, 16);
    for (int32_t i = 0; i < n; i++) {
        h = fnv(h, &v[i], 12);
        h = fnv(h, &v[i].type, 1);
    }
    return h;
}

static int run_stb(const uint8_t *d, size_t len, uint64_t *hash) {
    (void)len;
    void *font = stb_open(d, 0);
    if (!font) return REJECTED;
    for (int g = 0; g < stb_glyph_count(font); g++) {
        cff_vertex *v;
        int box[4];
        int n = stb_shape(font, g, &v);
        stb_box(font, g, box);
        *hash = glyph_hash(*hash, n, box, v);
        stb_free_shape(font, v);
    }
    return RAN;
}

static int run_c(const uint8_t *d, size_t len, uint64_t *hash) {
    const char *error;
    cff_font *font = cff_parse(d, len, 0, &error);
    if (!font) return REJECTED;
    cff_outline out = {0};
    for (int g = 0; g < cff_glyph_count(font); g++) {
        int box[4];
        int r = cff_glyph(font, g, &out, box, &error);
        *hash = glyph_hash(*hash, r < 0 ? -1 : (int32_t)out.len, box, out.v);
    }
    free(out.v);
    cff_free(font);
    return RAN;
}

static int run_rs(const uint8_t *d, size_t len, uint64_t *hash) { return rs_run(d, len, 0, hash); }

static int (*runs[3])(const uint8_t *, size_t, uint64_t *) = {run_stb, run_c, run_rs};

static int classify(int status) {
    if (WIFEXITED(status)) {
        switch (WEXITSTATUS(status)) {
        case RAN: return N_RAN;
        case REJECTED: return N_REJECTED;
        case PANICKED: return N_PANIC;
        case MEMORY: return N_MEMORY;
        }
        return N_OTHER;
    }
    if (WIFSIGNALED(status)) {
        switch (WTERMSIG(status)) {
        case SIGABRT: return N_ASSERT;
        case SIGALRM: return N_HANG;
        case SIGSEGV: case SIGBUS: case SIGFPE: case SIGILL: return N_CRASH;
        }
    }
    return N_OTHER;
}

/* Run one implementation on one font in a child; the child's ending. */
static int trial(int impl, const uint8_t *d, size_t len) {
    fflush(NULL);
    pid_t pid = fork();
    if (pid == 0) {
        /* Silence ASan, UBSan and assert reports; the exit status says it. */
        freopen("/dev/null", "w", stderr);
        alarm(3);
        hashes[impl] = 0xcbf29ce484222325ull;
        _exit(runs[impl](d, len, &hashes[impl]));
    }
    int status;
    waitpid(pid, &status, 0);
    return classify(status);
}

static uint64_t rng;
static uint32_t next(void) {
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return (uint32_t)(rng >> 16);
}

static uint32_t be32(const uint8_t *p) { return (uint32_t)p[0] << 24 | p[1] << 16 | p[2] << 8 | p[3]; }

/* The CFF table of face 0 of a plain sfnt. */
static int find_cff(const uint8_t *d, size_t len, size_t *at, size_t *size) {
    if (len < 12) return 0;
    size_t n = (size_t)(d[4] << 8 | d[5]);
    if (12 + 16 * n > len) return 0;
    for (size_t i = 0; i < n; i++) {
        const uint8_t *r = d + 12 + 16 * i;
        if (memcmp(r, "CFF ", 4) == 0 && (uint64_t)be32(r + 8) + be32(r + 12) <= len) {
            *at = be32(r + 8);
            *size = be32(r + 12);
            return 1;
        }
    }
    return 0;
}

static uint8_t *load(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    *len = (size_t)ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *d = malloc(*len);
    if (fread(d, 1, *len, f) != *len) d = NULL;
    fclose(f);
    return d;
}

int main(int argc, char **argv) {
    size_t len, at, size;
    hashes = mmap(NULL, 3 * sizeof *hashes, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (argc >= 3 && strcmp(argv[1], "one") == 0) {
        /* fuzz one <font>...: replay saved or hand-made fonts. */
        for (int a = 2; a < argc; a++) {
            uint8_t *d = load(argv[a], &len);
            if (!d) goto usage;
            printf("%s:", argv[a]);
            for (int i = 0; i < 3; i++) printf("  %s %s", impls[i], kinds[trial(i, d, len)]);
            printf("\n");
            free(d);
        }
        return 0;
    }
    if (argc < 4) goto usage;
    uint8_t *orig = load(argv[1], &len);
    if (!orig || !find_cff(orig, len, &at, &size)) {
        fprintf(stderr, "%s: no CFF table\n", argv[1]);
        return 1;
    }
    long mutants = atol(argv[2]);
    rng = (uint64_t)atol(argv[3]) * 0x9E3779B97F4A7C15ull | 1;
    const char *save = argc > 4 ? argv[4] : NULL;
    long counts[3][N_KINDS] = {{0}};
    int saved[3][N_KINDS] = {{0}};
    long same_c_rs = 0, same_stb = 0, all_ran = 0;
    uint8_t *d = malloc(len);
    for (long m = 0; m < mutants; m++) {
        memcpy(d, orig, len);
        int edits = 1 + next() % 8;
        for (int e = 0; e < edits; e++) {
            size_t p = at + next() % size;
            static const uint8_t special[] = {0, 1, 0x0b, 0x0e, 0x13, 0x1c, 0x1d, 0x1e, 0x1f, 0x7f, 0x80, 0xff};
            switch (next() % 3) {
            case 0: d[p] ^= (uint8_t)(1 << next() % 8); break;
            case 1: d[p] = (uint8_t)next(); break;
            default: d[p] = special[next() % sizeof special]; break;
            }
        }
        if (getenv("FUZZ_REASONS")) {
            /* Why cff.c turns mutants down, one line each. */
            const char *error;
            cff_font *font = cff_parse(d, len, 0, &error);
            if (font) cff_free(font);
            else printf("reason: %s\n", error);
            continue;
        }
        int ends[3];
        for (int i = 0; i < 3; i++) {
            int k = ends[i] = trial(i, d, len);
            counts[i][k]++;
            if (save && k >= N_PANIC && !saved[i][k]) {
                char path[512];
                snprintf(path, sizeof path, "%s/%s-%d.otf", save, impls[i], k);
                FILE *f = fopen(path, "wb");
                if (f) {
                    fwrite(d, 1, len, f);
                    fclose(f);
                }
                saved[i][k] = 1;
            }
        }
        if (ends[1] == N_RAN && ends[2] == N_RAN) {
            same_c_rs += hashes[1] == hashes[2];
            if (ends[0] == N_RAN) {
                all_ran++;
                same_stb += hashes[0] == hashes[1];
            }
        }
    }
    printf("%s: %ld mutants of a %zu-byte CFF table\n", argv[1], mutants, size);
    printf("  every glyph identical: cff.c = cff.rs in %ld of %ld both ran; = stb in %ld of %ld all three ran\n",
           same_c_rs, counts[1][N_RAN], same_stb, all_ran);
    for (int i = 0; i < 3; i++) {
        printf("  %-7s", impls[i]);
        for (int k = 0; k < N_KINDS; k++)
            if (counts[i][k]) printf("  %s %ld", kinds[k], counts[i][k]);
        printf("\n");
    }
    return 0;
usage:
    fprintf(stderr, "usage: fuzz <font> <mutants> <seed> [save-dir] | fuzz one <font>...\n");
    return 2;
}
