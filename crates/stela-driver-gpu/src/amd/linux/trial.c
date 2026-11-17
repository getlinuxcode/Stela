// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * The GPU trial's checks once amdgpu has bound a Radeon (AMD fork, J6):
 * Stela itself, not amdgpu's own tests, has the GPU run a command, and
 * lights the connected outputs through DCN. `stela_amdgpu_trial` runs on
 * the Linux task that probed the GPU (`engine/gpu_amd.rs`) and reports
 * each step on the console.
 *
 * - Command: a buffer of system memory (GTT) the GPU and the CPU both see,
 *   and on the first graphics ring and the first compute ring an indirect
 *   buffer with one PM4 WRITE_DATA of a value Stela picks to that buffer;
 *   the fence's signal and the value read back are the GPU's answer.
 * - Display: an in-kernel DRM client, as Linux's fbdev emulation is one:
 *   it probes the connectors, takes a dumb buffer as large as the modes it
 *   chose, fills it with colour bars, sets every chosen mode on it, and
 *   leaves it lit for DISPLAY_SECONDS so a person at the monitor sees it.
 */
#include <drm/drm_client.h>
#include <drm/drm_framebuffer.h>
#include <drm/drm_fourcc.h>
#include <drm/drm_modes.h>
#include <linux/delay.h>
#include <linux/dma-fence.h>
#include <linux/iosys-map.h>

#include "amdgpu.h"
#include "nvd.h"

#define DISPLAY_SECONDS 20
#define FENCE_TIMEOUT msecs_to_jiffies(2000)

/* One WRITE_DATA of `value` to `gpu_addr` on `ring`; 0 once the GPU wrote it. */
static long write_data(struct amdgpu_ring *ring, u64 gpu_addr, volatile u32 *cpu, u32 value)
{
	struct amdgpu_device *adev = ring->adev;
	struct dma_fence *fence = NULL;
	struct amdgpu_ib ib = {};
	long r;

	*cpu = 0;
	r = amdgpu_ib_get(adev, NULL, 16, AMDGPU_IB_POOL_DIRECT, &ib);
	if (r)
		return r;
	ib.ptr[0] = PACKET3(PACKET3_WRITE_DATA, 3);
	ib.ptr[1] = WRITE_DATA_DST_SEL(5) | WR_CONFIRM;
	ib.ptr[2] = lower_32_bits(gpu_addr);
	ib.ptr[3] = upper_32_bits(gpu_addr);
	ib.ptr[4] = value;
	ib.length_dw = 5;
	r = amdgpu_ib_schedule(ring, 1, &ib, NULL, &fence);
	if (!r) {
		r = dma_fence_wait_timeout(fence, false, FENCE_TIMEOUT);
		r = r == 0 ? -ETIMEDOUT : r < 0 ? r : (READ_ONCE(*cpu) == value ? 0 : -EIO);
	}
	amdgpu_ib_free(&ib, NULL);
	dma_fence_put(fence);
	return r;
}

static void command(struct amdgpu_device *adev)
{
	struct amdgpu_ring *rings[] = {
		adev->gfx.num_gfx_rings ? &adev->gfx.gfx_ring[0] : NULL,
		adev->gfx.num_compute_rings ? &adev->gfx.compute_ring[0] : NULL,
	};
	struct amdgpu_bo *bo = NULL;
	u64 gpu_addr;
	void *cpu;
	int r;

	r = amdgpu_bo_create_kernel(adev, PAGE_SIZE, PAGE_SIZE, AMDGPU_GEM_DOMAIN_GTT, &bo,
				    &gpu_addr, &cpu);
	if (r) {
		dev_err(adev->dev, "stela trial: no GTT buffer for the command (%d)\n", r);
		return;
	}
	for (unsigned int i = 0; i < ARRAY_SIZE(rings); i++) {
		struct amdgpu_ring *ring = rings[i];
		u32 value = 0x57e1a000 | i;
		ktime_t start;
		long result;

		if (!ring || !ring->sched.ready) {
			dev_info(adev->dev, "stela trial: no ready ring %u\n", i);
			continue;
		}
		start = ktime_get();
		result = write_data(ring, gpu_addr + i * 4, (u32 *)cpu + i, value);
		if (result)
			dev_err(adev->dev, "stela trial: ring %s: WRITE_DATA failed (%ld)\n",
				ring->name, result);
		else
			dev_info(adev->dev,
				 "stela trial: ring %s ran Stela's WRITE_DATA: 0x%08x at GTT 0x%llx, %lld us\n",
				 ring->name, value, gpu_addr + i * 4,
				 ktime_us_delta(ktime_get(), start));
	}
	amdgpu_bo_free_kernel(&bo, &gpu_addr, &cpu);
}

/* Eight vertical colour bars, as a test card's. */
static void bars(struct iosys_map *map, unsigned int width, unsigned int height,
		 unsigned int pitch)
{
	static const u32 colours[] = {
		0xffffff, 0xffff00, 0x00ffff, 0x00ff00, 0xff00ff, 0xff0000, 0x0000ff, 0x000000,
	};
	for (unsigned int y = 0; y < height; y++)
		for (unsigned int x = 0; x < width; x++)
			iosys_map_wr(map, (size_t)y * pitch + x * 4, u32,
				     colours[x * ARRAY_SIZE(colours) / width]);
}

static void display(struct amdgpu_device *adev)
{
	struct drm_device *dev = adev_to_drm(adev);
	struct drm_client_buffer *buffer;
	struct drm_client_dev client;
	struct drm_mode_set *modeset;
	unsigned int width = 0, height = 0, lit = 0;
	struct iosys_map map;
	int r;

	r = drm_client_init(dev, &client, "stela-trial", NULL);
	if (r) {
		dev_err(adev->dev, "stela trial: no DRM client (%d)\n", r);
		return;
	}
	r = drm_client_modeset_probe(&client, 0, 0);
	if (r) {
		dev_err(adev->dev, "stela trial: connector probe failed (%d)\n", r);
		goto release;
	}
	mutex_lock(&client.modeset_mutex);
	drm_client_for_each_modeset(modeset, &client) {
		if (!modeset->mode || !modeset->num_connectors)
			continue;
		dev_info(adev->dev, "stela trial: %s chose %ux%u@%u\n",
			 modeset->connectors[0]->name, modeset->mode->hdisplay,
			 modeset->mode->vdisplay, drm_mode_vrefresh(modeset->mode));
		width = max_t(unsigned int, width, modeset->mode->hdisplay);
		height = max_t(unsigned int, height, modeset->mode->vdisplay);
	}
	mutex_unlock(&client.modeset_mutex);
	if (!width) {
		dev_info(adev->dev, "stela trial: no connected output to light\n");
		goto release;
	}
	buffer = drm_client_buffer_create_dumb(&client, width, height, DRM_FORMAT_XRGB8888);
	if (IS_ERR(buffer)) {
		dev_err(adev->dev, "stela trial: no %ux%u buffer (%ld)\n", width, height,
			PTR_ERR(buffer));
		goto release;
	}
	r = drm_client_buffer_vmap(buffer, &map);
	if (r) {
		dev_err(adev->dev, "stela trial: buffer not mapped (%d)\n", r);
		goto delete;
	}
	bars(&map, width, height, buffer->fb->pitches[0]);
	mutex_lock(&client.modeset_mutex);
	drm_client_for_each_modeset(modeset, &client) {
		if (!modeset->mode)
			continue;
		modeset->fb = buffer->fb;
		modeset->x = 0;
		modeset->y = 0;
		lit++;
	}
	mutex_unlock(&client.modeset_mutex);
	r = drm_client_modeset_commit(&client);
	if (r) {
		dev_err(adev->dev, "stela trial: mode set failed (%d)\n", r);
	} else {
		dev_info(adev->dev,
			 "stela trial: %u output(s) lit through DCN with colour bars, %ux%u, for %u s\n",
			 lit, width, height, DISPLAY_SECONDS);
		msleep(DISPLAY_SECONDS * 1000);
	}
	drm_client_buffer_vunmap(buffer);
delete:
	drm_client_buffer_delete(buffer);
release:
	drm_client_release(&client);
}

/* The trial's checks on each Radeon amdgpu bound; how many it found. */
int stela_amdgpu_trial(void)
{
	unsigned int count;

	mutex_lock(&mgpu_info.mutex);
	count = mgpu_info.num_gpu;
	mutex_unlock(&mgpu_info.mutex);
	for (unsigned int i = 0; i < count; i++) {
		struct amdgpu_device *adev = mgpu_info.gpu_ins[i].adev;

		command(adev);
		display(adev);
	}
	return count;
}
