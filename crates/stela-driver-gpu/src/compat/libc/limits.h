/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef STELA_LIBC_LIMITS_H
#define STELA_LIBC_LIMITS_H
#include_next <limits.h>
#define PATH_MAX 4096
/* AMD fork (J7): RADV sizes its file-watch buffer by it. */
#define NAME_MAX 255
#endif
