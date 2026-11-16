/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * The kernel's floating-point section, for amdgpu's display core (AMD fork).
 *
 * Linux saves a task's FPU registers lazily and must claim them before
 * kernel code uses SSE. Stela keeps them across every switch: a fiber's
 * switch saves the x87 and SSE state (FXSAVE64, stela-boot-core's
 * fiber.rs), and a fiber gives its processor back only from a call of its
 * own, never in the middle of the display core's arithmetic. The FPU is
 * always available, and a section claims nothing.
 */
#ifndef _STELA_AMD_ASM_FPU_H
#define _STELA_AMD_ASM_FPU_H

#include <linux/types.h>

static inline bool kernel_fpu_available(void)
{
	return true;
}

static inline void kernel_fpu_begin(void)
{
}

static inline void kernel_fpu_end(void)
{
}

#endif
