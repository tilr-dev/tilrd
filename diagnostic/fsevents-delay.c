// Research probe: six simultaneous native streams, two paths, three writes each.
#include <CoreServices/CoreServices.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static char roots[2][1024];
static double written[2][3], start;
static int phase;
static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}
static void callback(ConstFSEventStreamRef s, void *info, size_t n,
                     void *paths, const FSEventStreamEventFlags flags[],
                     const FSEventStreamEventId ids[]) {
    (void)s;
    char **p = paths;
    for (size_t i = 0; i < n; i++) {
        for (int r = 0; r < 2; r++) {
            if (strncmp(p[i], roots[r], strlen(roots[r]))) continue;
            int marker = -1;
            const char *base = strrchr(p[i], '/');
            if (base) sscanf(base, "/marker-%d", &marker);
            double delay = marker >= 0 && marker < 3 && written[r][marker]
                         ? now() - written[r][marker] : -1;
            printf("EVENT t=%.6f stream=%ld root=%d marker=%d delay=%.6f flags=0x%x id=%llu path=%s\n",
                   now()-start, (long)info, r, marker, delay, flags[i], ids[i], p[i]);
        }
    }
}
static void tick(CFRunLoopTimerRef timer, void *info) {
    (void)timer; (void)info;
    if (phase < 3) {
        for (int r = 0; r < 2; r++) {
            char path[1200];
            snprintf(path, sizeof(path), "%s/marker-%d", roots[r], phase);
            written[r][phase] = now();
            int fd = open(path, O_WRONLY | O_CREAT | O_EXCL, 0600);
            if (fd < 0 || write(fd, "probe\n", 6) != 6 || fsync(fd) || close(fd)) {
                perror("write marker"); exit(2);
            }
            printf("WRITE t=%.6f root=%d marker=%d path=%s\n", now()-start,r,phase,path);
        }
        phase++;
    }
}
static void stop(CFRunLoopTimerRef timer, void *info) {
    (void)timer; (void)info;
    CFRunLoopStop(CFRunLoopGetCurrent());
}
int main(int argc, char **argv) {
    setbuf(stdout, NULL);
    start = now();
    snprintf(roots[0], sizeof(roots[0]), "/private/tmp/fsevents-delay-%d", getpid());
    char parent[1024];
    if (!realpath(argc > 1 ? argv[1] : "logs", parent)) { perror("realpath"); return 2; }
    snprintf(roots[1], sizeof(roots[1]), "%s/fsevents-delay-%d", parent, getpid());
    CFStringRef strings[2];
    for (int r=0;r<2;r++) {
        if (mkdir(roots[r],0700)) { perror("mkdir"); return 2; }
        strings[r]=CFStringCreateWithCString(NULL,roots[r],kCFStringEncodingUTF8);
    }
    CFArrayRef paths=CFArrayCreate(NULL,(const void **)strings,2,&kCFTypeArrayCallBacks);
    FSEventStreamRef streams[6];
    double latencies[]={0,0.001,0.2};
    for (long i=0;i<6;i++) {
        FSEventStreamContext ctx={0,(void *)i,NULL,NULL,NULL};
        unsigned flags=kFSEventStreamCreateFlagFileEvents | (i%2?kFSEventStreamCreateFlagNoDefer:0);
        streams[i]=FSEventStreamCreate(NULL,callback,&ctx,paths,kFSEventStreamEventIdSinceNow,latencies[i/2],flags);
        if (!streams[i]) return 2;
        FSEventStreamScheduleWithRunLoop(streams[i],CFRunLoopGetCurrent(),kCFRunLoopDefaultMode);
        Boolean ok=FSEventStreamStart(streams[i]);
        printf("START stream=%ld latency=%.3f nodefer=%ld ok=%d wall=%lld\n",i,latencies[i/2],i%2,ok,(long long)time(NULL));
        if (!ok) return 2;
    }
    CFRunLoopTimerContext ctx={0,NULL,NULL,NULL,NULL};
    CFRunLoopTimerRef writer=CFRunLoopTimerCreate(NULL,CFAbsoluteTimeGetCurrent()+2,5,0,0,tick,&ctx);
    CFRunLoopTimerRef end=CFRunLoopTimerCreate(NULL,CFAbsoluteTimeGetCurrent()+90,0,0,0,stop,&ctx);
    CFRunLoopAddTimer(CFRunLoopGetCurrent(),writer,kCFRunLoopDefaultMode);
    CFRunLoopAddTimer(CFRunLoopGetCurrent(),end,kCFRunLoopDefaultMode);
    CFRunLoopRun();
    for(int i=0;i<6;i++) { FSEventStreamStop(streams[i]); FSEventStreamInvalidate(streams[i]); FSEventStreamRelease(streams[i]); }
    printf("END t=%.6f\n",now()-start);
    return 0;
}
