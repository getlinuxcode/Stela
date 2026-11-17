// SPDX-License-Identifier: GPL-2.0-or-later
/* AMD fork (J7): RADV's hang report names the system (`uname`), which the
 * image has none of to name; the call refuses (`mesa/radv_libc.c`). */
#ifndef STELA_SYS_UTSNAME_H
#define STELA_SYS_UTSNAME_H

struct utsname {
	char sysname[65];
	char nodename[65];
	char release[65];
	char version[65];
	char machine[65];
	char domainname[65];
};

int uname(struct utsname *name);

#endif
