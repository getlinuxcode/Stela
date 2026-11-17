// SPDX-License-Identifier: GPL-2.0-or-later
/* AMD fork (J7): RADV watches a trigger file for its thread trace, which
 * the image has no file system for; the calls refuse (`mesa/radv_libc.c`). */
#ifndef STELA_SYS_INOTIFY_H
#define STELA_SYS_INOTIFY_H

#include <stdint.h>

#define IN_MODIFY 0x00000002
#define IN_DELETE_SELF 0x00000400
#define IN_NONBLOCK 04000
#define IN_CLOEXEC 02000000

struct inotify_event {
	int wd;
	uint32_t mask;
	uint32_t cookie;
	uint32_t len;
	char name[];
};

int inotify_init1(int flags);
int inotify_add_watch(int fd, const char *path, uint32_t mask);
int inotify_rm_watch(int fd, int watch);

#endif
