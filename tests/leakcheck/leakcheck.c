// Test-only LD_PRELOAD shim: appends one line to $LEAKCHECK_LOG for every name lookup and every network
// destination the process uses. It only observes; behaviour is unchanged. Not built by cargo.
//   getaddrinfo <name> <service>      gethostbyname <name>
//   connect|sendto|sendmsg|sendmmsg <ip> <port>      connect unix <path>
// Limits: calls that bypass libc's exported symbols (raw syscalls, glibc's internal resolver sends) are not seen;
// a name lookup is caught at getaddrinfo, a direct DNS packet only when the program sends it itself.
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <netdb.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static void put(const char *call, const char *what) {
    const char *path = getenv("LEAKCHECK_LOG");
    if (!path) return;
    char line[512];
    int n = snprintf(line, sizeof line, "%s %s\n", call, what);
    int fd = open(path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0600);
    if (fd < 0) return;
    if (n > 0) (void)!write(fd, line, n < (int)sizeof line ? n : (int)sizeof line - 1);
    close(fd);
}

static void dest(const char *call, const struct sockaddr *sa) {
    char ip[INET6_ADDRSTRLEN], what[300];
    if (!sa) return;
    if (sa->sa_family == AF_INET) {
        const struct sockaddr_in *a = (const void *)sa;
        inet_ntop(AF_INET, &a->sin_addr, ip, sizeof ip);
        snprintf(what, sizeof what, "%s %u", ip, ntohs(a->sin_port));
    } else if (sa->sa_family == AF_INET6) {
        const struct sockaddr_in6 *a = (const void *)sa;
        inet_ntop(AF_INET6, &a->sin6_addr, ip, sizeof ip);
        snprintf(what, sizeof what, "%s %u", ip, ntohs(a->sin6_port));
    } else if (sa->sa_family == AF_UNIX) {
        const struct sockaddr_un *a = (const void *)sa;
        snprintf(what, sizeof what, "unix %.100s", a->sun_path[0] ? a->sun_path : a->sun_path + 1);
    } else {
        return;
    }
    put(call, what);
}

#define REAL(ret, name, args) \
    static ret(*real)args; \
    if (!real) real = dlsym(RTLD_NEXT, name)

int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **res) {
    REAL(int, "getaddrinfo", (const char *, const char *, const struct addrinfo *, struct addrinfo **));
    char what[300];
    snprintf(what, sizeof what, "%.200s %.50s", node ? node : "(null)", service ? service : "(null)");
    put("getaddrinfo", what);
    return real(node, service, hints, res);
}

struct hostent *gethostbyname(const char *name) {
    REAL(struct hostent *, "gethostbyname", (const char *));
    put("gethostbyname", name ? name : "(null)");
    return real(name);
}

struct hostent *gethostbyname2(const char *name, int af) {
    REAL(struct hostent *, "gethostbyname2", (const char *, int));
    put("gethostbyname", name ? name : "(null)");
    return real(name, af);
}

int connect(int fd, const struct sockaddr *sa, socklen_t len) {
    REAL(int, "connect", (int, const struct sockaddr *, socklen_t));
    dest("connect", sa);
    return real(fd, sa, len);
}

ssize_t sendto(int fd, const void *buf, size_t n, int flags, const struct sockaddr *sa, socklen_t len) {
    REAL(ssize_t, "sendto", (int, const void *, size_t, int, const struct sockaddr *, socklen_t));
    dest("sendto", sa);
    return real(fd, buf, n, flags, sa, len);
}

ssize_t sendmsg(int fd, const struct msghdr *m, int flags) {
    REAL(ssize_t, "sendmsg", (int, const struct msghdr *, int));
    dest("sendmsg", m ? m->msg_name : NULL);
    return real(fd, m, flags);
}

int sendmmsg(int fd, struct mmsghdr *v, unsigned int n, int flags) {
    REAL(int, "sendmmsg", (int, struct mmsghdr *, unsigned int, int));
    for (unsigned int i = 0; v && i < n; i++) dest("sendmmsg", v[i].msg_hdr.msg_name);
    return real(fd, v, n, flags);
}
