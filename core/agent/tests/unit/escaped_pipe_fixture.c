// Native child/pipe fixture. Setup is compiled and probed before timing starts.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static double now(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value) != 0) {
        perror("clock_gettime");
        exit(2);
    }
    return value.tv_sec + value.tv_nsec / 1000000000.0;
}

static void mark(const char *path, const char *text) {
    FILE *file = fopen(path, "w");
    if (!file) {
        perror(path);
        exit(2);
    }
    fputs(text, file);
    fclose(file);
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--probe") == 0) return 0;
    if (argc != 3 || chdir(argv[1]) != 0) return 2;
    mark("phase", "native_started");
    pid_t child = fork();
    if (child < 0) { perror("fork"); return 2; }
    if (child == 0) {
        if (setsid() < 0) { perror("setsid"); _exit(2); }
        mark("ready", "");
        double deadline = now() + 8.0;
        while (access("release", F_OK) != 0 && now() < deadline) usleep(10000);
        mark("done", "");
        _exit(0);
    }
    double deadline = now() + 3.0;
    while (access("ready", F_OK) != 0 && now() < deadline) usleep(10000);
    if (access("ready", F_OK) != 0) return 3;
    puts("captured");
    fflush(stdout);
    if (strcmp(argv[2], "keep") == 0) sleep(8);
    return 0;
}
