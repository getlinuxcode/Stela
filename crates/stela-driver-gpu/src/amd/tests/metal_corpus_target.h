/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef STELA_AMD_METAL_CORPUS_TARGET_H
#define STELA_AMD_METAL_CORPUS_TARGET_H
/* The AMD target of stela_metal_corpus (--gpu FAMILY, default GFX1201): the
 * NIR options RADV hands its ACO backend, and the limits the Metal device
 * derives from RADV's Vulkan limits on an RX 9070 XT (metal/compiler.rs,
 * docs/amd/qualification.md). A CPU compiler model, never a device. */
#include "amd/common/ac_gpu_info.h"
#include "amd/common/nir/ac_nir.h"
#include "amd/compiler/aco_interface.h"
#include <stdio.h>
#include <strings.h>

#define STELA_CORPUS_TARGET_OPTION "--gpu"
#define STELA_CORPUS_TARGET_DEFAULT "gfx1201"

static struct radeon_info stela_corpus_gpu;
static nir_shader_compiler_options stela_corpus_nir;

/* Fills target for the family named name; false after reporting why. */
static bool
stela_corpus_target_open(const char *name, struct stela_metal_target *target)
{
   enum radeon_family family = CHIP_UNKNOWN;
   for (unsigned i = CHIP_TAHITI; i < CHIP_LAST; i++) {
      if (!strcasecmp(ac_get_family_name((enum radeon_family)i), name))
         family = (enum radeon_family)i;
   }
   if (family == CHIP_UNKNOWN) {
      fprintf(stderr, "unknown AMD family %s\n", name);
      return false;
   }
   stela_corpus_gpu = (struct radeon_info){
      .gfx_level = ac_get_gfx_level(family), .family = family, .has_graphics = true,
   };
   ac_fill_compiler_info(&stela_corpus_gpu, NULL, false);
   if (!aco_is_gpu_supported(&stela_corpus_gpu)) {
      fprintf(stderr, "ACO does not compile %s\n", ac_get_family_name(family));
      return false;
   }
   ac_nir_set_options(&stela_corpus_gpu.compiler_info, false, &stela_corpus_nir);
   *target = (struct stela_metal_target){
      .nir = &stela_corpus_nir, .max_instructions = 1u << 20,
      /* As metal/compiler.rs clamps maxComputeSharedMemorySize (65536). */
      .max_threadgroup_bytes = 65535, .max_private_bytes = 512u << 10,
      /* RADV's maxUniformBufferRange. */
      .max_constant_bytes = 0xffffffffu,
      /* maxVertexOutputComponents and maxFragmentInputComponents are 128. */
      .max_varying_slots = 32,
      .max_payload_bytes = 16384, .max_mesh_vertices = 256, .max_mesh_primitives = 256,
      .stencil_export = 1, .raster_order_groups = 1, .fragment_tests = 1,
   };
   printf("target %s, RADV NIR options\n", ac_get_family_name(family));
   return true;
}

static void
stela_corpus_target_close(void)
{
}

#endif
