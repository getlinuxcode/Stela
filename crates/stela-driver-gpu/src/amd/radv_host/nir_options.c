/* SPDX-License-Identifier: GPL-2.0-or-later */
/* The NIR options the AIR compiler builds a RADV device's shaders with, which
 * Vulkan offers no route to; RADV's counterpart of stela_nvk_nir_options.
 *
 * RADV keeps per-stage options on the logical device (radv_get_nir_options);
 * the Metal compiler takes one set per physical device, so this derives the
 * compute stage's from the same facts. They differ from the other stages'
 * only in preferring split multiply-adds where RADV's key asks for it. */
#include "radv_physical_device.h"
#include "ac_nir.h"
#include "util/simple_mtx.h"

#define DEVICES 4

static simple_mtx_t lock = SIMPLE_MTX_INITIALIZER;
static struct {
   const struct radv_physical_device *pdev;
   nir_shader_compiler_options options;
} devices[DEVICES];

PUBLIC const struct nir_shader_compiler_options *
stela_radv_nir_options(VkPhysicalDevice physical);

PUBLIC const struct nir_shader_compiler_options *
stela_radv_nir_options(VkPhysicalDevice physical)
{
   VK_FROM_HANDLE(radv_physical_device, pdev, physical);
   const struct nir_shader_compiler_options *found = NULL;
   simple_mtx_lock(&lock);
   for (unsigned i = 0; i < DEVICES && !found; i++) {
      if (devices[i].pdev == pdev) {
         found = &devices[i].options;
      } else if (!devices[i].pdev) {
         nir_shader_compiler_options *options = &devices[i].options;
         ac_nir_set_options(&pdev->info.compiler_info, pdev->use_llvm, options);
         /* As get_nir_options_for_stage in radv_shader.c. */
         options->max_unroll_iterations = 32;
         options->max_unroll_iterations_aggressive = 128;
         options->lower_doubles_options =
            nir_lower_drcp | nir_lower_dsqrt | nir_lower_drsq | nir_lower_ddiv;
         options->frag_coord_form = nir_frag_coord_xy_z_w_separate | nir_frag_coord_use_w_rcp;
         options->io_options |= nir_io_mediump_is_32bit | nir_io_radv_intrinsic_component_workaround;
         options->varying_expression_max_cost = ac_nir_varying_expression_max_cost;
         devices[i].pdev = pdev;
         found = options;
      }
   }
   simple_mtx_unlock(&lock);
   return found;
}
