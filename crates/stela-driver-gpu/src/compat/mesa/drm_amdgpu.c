// SPDX-License-Identifier: GPL-2.0-or-later
/* AMD fork (J7): the entry points of libdrm's core that libdrm_amdgpu calls
 * and that `drm.c` (as far as NVK reaches libdrm) does not have. Stela opens
 * render nodes only (`compat/libc` `open`), each a bound GPU's, numbered from
 * DRM's render minor 128 by the GPU's index. */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <xf86drm.h>

/* Every descriptor is a render node's. */
int drmGetNodeTypeFromFd(int fd)
{
	struct stat status;

	if (fstat(fd, &status) || major(status.st_rdev) != 226)
		return -1;
	return DRM_NODE_RENDER;
}

/* The GPU's primary node, by its render node's index: libdrm_amdgpu tells
 * two descriptors of one GPU by it (amdgpu_device_initialize). */
char *drmGetPrimaryDeviceNameFromFd(int fd)
{
	struct stat status;
	char *name;

	if (fstat(fd, &status) || major(status.st_rdev) != 226 || minor(status.st_rdev) < 128)
		return NULL;
	if (asprintf(&name, "/dev/dri/card%u", minor(status.st_rdev) - 128) < 0)
		return NULL;
	return name;
}

/* libdrm's debug messages, which it prints only when asked to. */
void drmMsg(const char *format, ...)
{
	(void)format;
}

int drmSyncobjQuery(int fd, uint32_t *handles, uint64_t *points, uint32_t count)
{
	return drmSyncobjQuery2(fd, handles, points, count, 0);
}

/* libdrm_amdgpu's marketing names come from a table file
 * (`amdgpu_asic_id.c`), which the image has none of: no GPU gets a
 * libdrm name, and RADV names it from its own tables. */
struct amdgpu_device;
void amdgpu_parse_asic_ids(struct amdgpu_device *dev);
void amdgpu_parse_asic_ids(struct amdgpu_device *dev)
{
	(void)dev;
}
