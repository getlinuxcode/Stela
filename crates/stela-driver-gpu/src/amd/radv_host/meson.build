# SPDX-License-Identifier: GPL-2.0-or-later
# libstela_radv: RADV, built from Stela's Mesa, with the AIR compiler service
# linked into the same library, as NVK and the compiler share stela.efi. The
# Metal device hands RADV the NIR the compiler builds (through Mesa's internal
# VkPipelineShaderStageNirCreateInfoMESA), so both must be one Mesa: NIR
# shaders point into the GLSL type tables of the library that built them.
# For running Stela's Metal device on Linux (stela-amd-host --device).
radv = {}
foreach name : ['libradv_files', 'radv_entrypoints', 'radv_drirc', 'sha1_h', 'radix_sort_spv',
                'bvh_spv', 'radv_annotate_layer', 'radv_flags', 'radv_deps',
                'idep_radv_tracepoints']
  radv += {name : mesa.get_variable(name)}
endforeach
foreach name : ['inc_include', 'inc_src', 'inc_amd', 'inc_amd_common', 'inc_amd_common_llvm',
                'inc_util', 'inc_virtio_gpu', 'libamd_common', 'libamd_common_llvm',
                'libamdgpu_addrlib', 'dep_llvm', 'dep_libdrm_amdgpu', 'dep_thread', 'dep_elf',
                'dep_dl', 'dep_m', 'dep_valgrind', 'idep_aco', 'idep_mesautil', 'idep_nir',
                'idep_vulkan_util', 'idep_vulkan_wsi', 'idep_vulkan_runtime',
                'idep_amd_generated_headers', 'idep_xmlconfig', 'no_override_init_args',
                'ld_args_build_id', 'ld_args_bsymbolic', 'ld_args_gc_sections']
  radv += {name : mesa.get_variable(name)}
endforeach

symbols = files('stela_radv.sym')
stela_radv = shared_library(
  'stela_radv',
  [radv['libradv_files'], radv['radv_entrypoints'], radv['radv_drirc'], radv['sha1_h'],
   radv['radix_sort_spv'], radv['bvh_spv'], radv['radv_annotate_layer'], 'nir_options.c'],
  include_directories : [
    radv['inc_include'], radv['inc_src'], radv['inc_amd'], radv['inc_amd_common'],
    radv['inc_amd_common_llvm'], radv['inc_util'], radv['inc_virtio_gpu'],
    include_directories(mesa_src / 'amd/vulkan'),
  ],
  link_with : [radv['libamd_common'], radv['libamd_common_llvm'], radv['libamdgpu_addrlib']],
  link_whole : [stela_metal_compiler, stela_metal_abi],
  dependencies : [
    radv['dep_llvm'], radv['dep_libdrm_amdgpu'], radv['dep_thread'], radv['dep_elf'],
    radv['dep_dl'], radv['dep_m'], radv['dep_valgrind'], radv['radv_deps'], radv['idep_aco'],
    radv['idep_mesautil'], radv['idep_nir'], radv['idep_vulkan_util'], radv['idep_vulkan_wsi'],
    radv['idep_vulkan_runtime'], radv['idep_amd_generated_headers'], radv['idep_xmlconfig'],
    radv['idep_radv_tracepoints'], stela_air_test_dependencies,
  ],
  # RADV's sources compile as Mesa builds them, not under Stela's -Werror.
  c_args : [radv['no_override_init_args'], radv['radv_flags'], '-Wno-error'],
  cpp_args : [radv['radv_flags'], '-Wno-error'],
  link_args : [radv['ld_args_build_id'], radv['ld_args_bsymbolic'], radv['ld_args_gc_sections'],
               '-Wl,--version-script,' + (meson.current_source_dir() / 'stela_radv.sym')],
  link_depends : symbols,
  build_rpath : fs.parent(stela_air_llvm),
)
