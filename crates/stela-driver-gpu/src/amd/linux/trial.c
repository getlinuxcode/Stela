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
 * - Screen: an in-kernel DRM client, as Linux's fbdev emulation is one,
 *   probes the connectors, takes a dumb buffer as large as the modes it
 *   chose, clears it once amdgpu's own clear has ended, and sets every
 *   chosen mode on it. Its mapping of VRAM lies in the host's page window,
 *   which every processor sees, so Stela's console draws there
 *   (`stela_amdgpu_screen`, `engine/gpu_amd.rs`). The colour bars a
 *   seventh trial showed on the monitor came this way.
 */
#include <drm/drm_client.h>
#include <drm/drm_framebuffer.h>
#include <drm/drm_fourcc.h>
#include <drm/drm_modes.h>
#include <linux/delay.h>
#include <linux/dma-resv.h>
#include <linux/dma-fence.h>
#include <linux/iosys-map.h>

#include "amdgpu.h"
#include "amdgpu_dm.h"
#include "dc.h"
#include "nvd.h"

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

/*
 * What the display reached: each lit CRTC's hardware frame counter over a
 * second and each lit DisplayPort link's trained lanes and rate.
 */
static void observe(struct amdgpu_device *adev, struct drm_client_dev *client)
{
	struct drm_mode_set *modeset;

	mutex_lock(&client->modeset_mutex);
	drm_client_for_each_modeset(modeset, client) {
		struct drm_crtc *crtc = modeset->crtc;

		if (!modeset->mode)
			continue;
		if (crtc->funcs->get_vblank_counter) {
			u32 before = crtc->funcs->get_vblank_counter(crtc);

			msleep(1000);
			dev_info(adev->dev, "stela console: CRTC %u counted %u frames in 1 s\n",
				 crtc->index, crtc->funcs->get_vblank_counter(crtc) - before);
		}
		for (unsigned int c = 0; c < modeset->num_connectors; c++) {
			struct drm_connector *connector = modeset->connectors[c];
			struct dc_link *link = to_amdgpu_dm_connector(connector)->dc_link;

			if (link)
				dev_info(adev->dev, "stela console: %s link: %u lanes at rate 0x%x\n",
					 connector->name, link->cur_link_settings.lane_count,
					 link->cur_link_settings.link_rate);
		}
	}
	mutex_unlock(&client->modeset_mutex);
}

/*
 * The screen Stela's console takes: the client, its buffer and mapping
 * live for the boot, as Linux's fbdev emulation keeps its own.
 */
struct stela_amdgpu_screen {
	void *pixels;
	u32 width;
	u32 height;
	u32 pitch;
	u32 refresh_millihertz;
};

static struct drm_client_dev screen_client;
static struct stela_amdgpu_screen screen_found;

static void screen(struct amdgpu_device *adev)
{
	struct drm_device *dev = adev_to_drm(adev);
	struct drm_client_dev *client = &screen_client;
	struct drm_client_buffer *buffer;
	struct drm_mode_set *modeset;
	unsigned int width = 0, height = 0, refresh = 0, lit = 0;
	struct iosys_map map;
	long waited;
	int r;

	r = drm_client_init(dev, client, "stela-console", NULL);
	if (r) {
		dev_err(adev->dev, "stela console: no DRM client (%d)\n", r);
		return;
	}
	r = drm_client_modeset_probe(client, 0, 0);
	if (r) {
		dev_err(adev->dev, "stela console: connector probe failed (%d)\n", r);
		goto release;
	}
	mutex_lock(&client->modeset_mutex);
	drm_client_for_each_modeset(modeset, client) {
		if (!modeset->mode || !modeset->num_connectors)
			continue;
		dev_info(adev->dev, "stela console: %s chose %ux%u@%u\n",
			 modeset->connectors[0]->name, modeset->mode->hdisplay,
			 modeset->mode->vdisplay, drm_mode_vrefresh(modeset->mode));
		width = max_t(unsigned int, width, modeset->mode->hdisplay);
		height = max_t(unsigned int, height, modeset->mode->vdisplay);
		if (!refresh)
			refresh = drm_mode_vrefresh(modeset->mode);
	}
	mutex_unlock(&client->modeset_mutex);
	if (!width) {
		dev_info(adev->dev, "stela console: no connected output\n");
		goto release;
	}
	buffer = drm_client_buffer_create_dumb(client, width, height, DRM_FORMAT_XRGB8888);
	if (IS_ERR(buffer)) {
		dev_err(adev->dev, "stela console: no %ux%u buffer (%ld)\n", width, height,
			PTR_ERR(buffer));
		goto release;
	}
	r = drm_client_buffer_vmap(buffer, &map);
	if (r || !map.is_iomem) {
		dev_err(adev->dev, "stela console: buffer not mapped as device memory (%d)\n", r);
		goto delete;
	}
	/*
	 * amdgpu clears a new VRAM buffer with SDMA, a job that may still run:
	 * what the CPU writes before it ends is cleared away (the sixth trial
	 * read black back). Wait for every fence on the buffer first.
	 */
	waited = dma_resv_wait_timeout(buffer->fb->obj[0]->resv, DMA_RESV_USAGE_BOOKKEEP, false,
				       msecs_to_jiffies(2000));
	if (waited <= 0) {
		dev_err(adev->dev, "stela console: buffer still busy (%ld)\n", waited);
		goto unmap;
	}
	iosys_map_memset(&map, 0, 0, (size_t)buffer->fb->pitches[0] * height);
	mutex_lock(&client->modeset_mutex);
	drm_client_for_each_modeset(modeset, client) {
		if (!modeset->mode)
			continue;
		modeset->fb = buffer->fb;
		modeset->x = 0;
		modeset->y = 0;
		lit++;
	}
	mutex_unlock(&client->modeset_mutex);
	r = drm_client_modeset_commit(client);
	if (r) {
		dev_err(adev->dev, "stela console: mode set failed (%d)\n", r);
		goto unmap;
	}
	observe(adev, client);
	screen_found = (struct stela_amdgpu_screen){
		.pixels = (void __force *)map.vaddr_iomem,
		.width = width,
		.height = height,
		.pitch = buffer->fb->pitches[0],
		.refresh_millihertz = refresh * 1000,
	};
	dev_info(adev->dev, "stela console: %u output(s) lit, %ux%u, pitch %u, for Stela's console\n",
		 lit, width, height, buffer->fb->pitches[0]);
	return;
unmap:
	drm_client_buffer_vunmap(buffer);
delete:
	drm_client_buffer_delete(buffer);
release:
	drm_client_release(client);
}

/* The screen the trial lit for Stela's console; false when none was. */
bool stela_amdgpu_screen(struct stela_amdgpu_screen *out)
{
	if (!screen_found.pixels)
		return false;
	*out = screen_found;
	return true;
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
		if (i == 0)
			screen(adev);
	}
	return count;
}
