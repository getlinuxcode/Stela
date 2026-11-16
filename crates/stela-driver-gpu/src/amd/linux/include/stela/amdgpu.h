/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * What amdgpu's sources reach that Stela's Linux configuration leaves out
 * (AMD fork), included before each of them.
 */
#ifndef _STELA_AMDGPU_H
#define _STELA_AMDGPU_H

struct irq_domain;

/*
 * amdgpu's interrupt domain forwards its sources to child devices (the
 * audio coprocessor and image signal processor), which Stela's amdgpu
 * leaves out: no interrupt has a handler there.
 */
static inline int generic_handle_domain_irq(struct irq_domain *domain, unsigned int hwirq)
{
	(void)domain;
	(void)hwirq;
	return -22; /* -EINVAL */
}

struct fwnode_handle;
struct irq_domain_ops;

/*
 * Only older interrupt handlers (Tonga, Iceland, CIK, CZ) and RDNA 3.5's
 * ih_v6_1 make that domain; RDNA 4's ih_v7_0 never does.
 */
static inline struct irq_domain *irq_domain_create_linear(struct fwnode_handle *fwnode,
							   unsigned int size,
							   const struct irq_domain_ops *ops,
							   void *host_data)
{
	(void)fwnode;
	(void)size;
	(void)ops;
	(void)host_data;
	return (struct irq_domain *)0;
}

static inline void irq_domain_remove(struct irq_domain *domain)
{
	(void)domain;
}

/* Only the audio coprocessor maps its interrupts there: no Linux IRQ. */
static inline unsigned int irq_create_mapping(struct irq_domain *domain, unsigned long hwirq)
{
	(void)domain;
	(void)hwirq;
	return 0;
}

/* One memory node: Stela does not run NUMA. */
#define pfn_to_nid(pfn) ((void)(pfn), 0)

#endif
