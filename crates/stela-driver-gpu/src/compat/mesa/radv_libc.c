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
