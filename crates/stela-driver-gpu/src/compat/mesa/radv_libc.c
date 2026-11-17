// SPDX-License-Identifier: GPL-2.0-or-later
/* AMD fork (J7): the C library calls RADV and ACO make for their debugging
 * tools, which the image has nothing behind: no shell, no system name, no
 * file system to watch. Each refuses, which RADV takes as the tool being
 * unavailable. */
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <sys/inotify.h>
#include <sys/utsname.h>

int system(const char *command)
{
	(void)command;
	errno = ENOSYS;
	return -1;
}

int uname(struct utsname *name)
{
	(void)name;
	errno = ENOSYS;
	return -1;
}

int inotify_init1(int flags)
{
	(void)flags;
	errno = ENOSYS;
	return -1;
}

int inotify_add_watch(int fd, const char *path, uint32_t mask)
{
	(void)fd;
	(void)path;
	(void)mask;
	errno = EBADF;
	return -1;
}

int inotify_rm_watch(int fd, int watch)
{
	(void)fd;
	(void)watch;
	errno = EBADF;
	return -1;
}

/*
 * What `__builtin_cpu_supports` reads (compiler-rt's), which AMD's address
 * library asks to pick an AVX2 copy of tiled images. Nothing initializes
 * it in the image, so every feature reads absent and the plain copy runs.
 */
struct stela_cpu_model {
	unsigned int vendor;
	unsigned int type;
	unsigned int subtype;
	unsigned int features[1];
};
struct stela_cpu_model __cpu_model;
unsigned int __cpu_features2[3];
