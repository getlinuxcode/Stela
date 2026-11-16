// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * PCI, DMA, memory and time services amdgpu reaches that Stela's Linux
 * environment does not define (AMD fork, J6). Capabilities and link state
 * are read from the card's configuration space; what reconfigures the bus
 * (BAR resizing, function resets, atomics routing) is refused, as Stela
 * keeps the firmware's assignment.
 */
#include <linux/bitfield.h>
#include <linux/dma-mapping.h>
#include <linux/ioport.h>
#include <linux/log2.h>
#include <linux/pci.h>
#include <linux/timekeeping.h>

/* Extended capabilities: the list from offset 0x100, in 4 KiB of space. */
u16 pci_find_ext_capability(struct pci_dev *dev, int cap)
{
	int position = PCI_CFG_SPACE_SIZE;
	/* At most one capability per 8 bytes of extended space. */
	int left = (PCI_CFG_SPACE_EXP_SIZE - PCI_CFG_SPACE_SIZE) / 8;
	u32 header;

	if (pci_read_config_dword(dev, position, &header) || header == 0 || header == ~0u)
		return 0;
	while (left-- > 0) {
		if (PCI_EXT_CAP_ID(header) == cap && position != 0)
			return position;
		position = PCI_EXT_CAP_NEXT(header);
		if (position < PCI_CFG_SPACE_SIZE)
			return 0;
		if (pci_read_config_dword(dev, position, &header))
			return 0;
	}
	return 0;
}

static int express(struct pci_dev *dev)
{
	return dev->pcie_cap ? dev->pcie_cap : pci_find_capability(dev, PCI_CAP_ID_EXP);
}

int pcie_capability_read_word(struct pci_dev *dev, int pos, u16 *val)
{
	int cap = express(dev);

	*val = 0;
	if (!cap || pos & 1)
		return -EINVAL;
	return pci_read_config_word(dev, cap + pos, val);
}

static u32 link_capabilities(struct pci_dev *dev)
{
	int cap = express(dev);
	u32 value = 0;

	if (cap)
		pci_read_config_dword(dev, cap + PCI_EXP_LNKCAP, &value);
	return value;
}

/* A link speed code (LNKCAP's and LNKSTA's 1 to 6) as Linux's speed. */
static enum pci_bus_speed speed_of(unsigned int code)
{
	static const enum pci_bus_speed speeds[] = {
		PCI_SPEED_UNKNOWN, PCIE_SPEED_2_5GT, PCIE_SPEED_5_0GT, PCIE_SPEED_8_0GT,
		PCIE_SPEED_16_0GT, PCIE_SPEED_32_0GT, PCIE_SPEED_64_0GT,
	};

	return code < ARRAY_SIZE(speeds) ? speeds[code] : PCI_SPEED_UNKNOWN;
}

/* Usable megabits per second per lane, after the line encoding. */
static u32 lane_mbps(unsigned int code)
{
	static const u32 rates[] = { 0, 2000, 4000, 7877, 15754, 31508, 62016 };

	return code < ARRAY_SIZE(rates) ? rates[code] : 0;
}

/* The fastest speed the link supports (LNKCAP2's vector, else LNKCAP). */
enum pci_bus_speed pcie_get_speed_cap(struct pci_dev *dev)
{
	int cap = express(dev);
	u32 vector = 0;

	if (cap)
		pci_read_config_dword(dev, cap + PCI_EXP_LNKCAP2, &vector);
	vector &= PCI_EXP_LNKCAP2_SLS;
	if (vector)
		return speed_of(fls(vector) - 1);
	return speed_of(link_capabilities(dev) & PCI_EXP_LNKCAP_SLS);
}

enum pcie_link_width pcie_get_width_cap(struct pci_dev *dev)
{
	return FIELD_GET(PCI_EXP_LNKCAP_MLW, link_capabilities(dev));
}

/*
 * The bandwidth the card's own link runs at. Linux takes the narrowest
 * link up to the root; Stela reads the card's, which amdgpu only reports.
 */
u32 pcie_bandwidth_available(struct pci_dev *dev, struct pci_dev **limiting_dev,
			     enum pci_bus_speed *speed, enum pcie_link_width *width)
{
	u16 status = 0;
	unsigned int code, lanes;

	pcie_capability_read_word(dev, PCI_EXP_LNKSTA, &status);
	code = status & PCI_EXP_LNKSTA_CLS;
	lanes = FIELD_GET(PCI_EXP_LNKSTA_NLW, status);
	if (limiting_dev)
		*limiting_dev = dev;
	if (speed)
		*speed = speed_of(code);
	if (width)
		*width = lanes;
	return lanes * lane_mbps(code);
}

int pcie_get_mps(struct pci_dev *dev)
{
	u16 control = 0;

	pcie_capability_read_word(dev, PCI_EXP_DEVCTL, &control);
	return 128 << FIELD_GET(PCI_EXP_DEVCTL_PAYLOAD, control);
}

void pci_clear_master(struct pci_dev *dev)
{
	u16 command;

	if (pci_read_config_word(dev, PCI_COMMAND, &command))
		return;
	if (command & PCI_COMMAND_MASTER)
		pci_write_config_word(dev, PCI_COMMAND, command & ~PCI_COMMAND_MASTER);
	dev->is_busmaster = 0;
}

/*
 * Stela publishes the card it drives, not the bus: no other device of a
 * class is found (amdgpu looks for a laptop's other GPU and its audio).
 */
struct pci_dev *pci_get_class(unsigned int class, struct pci_dev *from)
{
	(void)class;
	pci_dev_put(from);
	return NULL;
}

/* Atomics routed to the root serve the compute API (KFD) only. */
int pci_enable_atomic_ops_to_root(struct pci_dev *dev, u32 cap_mask)
{
	(void)dev;
	(void)cap_mask;
	return -EINVAL;
}

/*
 * Bus reconfiguration: Stela keeps the firmware's assignment. Without a
 * root window above 4 GiB amdgpu leaves the VRAM BAR as the firmware sized
 * it, the whole VRAM when Resizable BAR is on.
 */
struct resource *pci_bus_resource_n(const struct pci_bus *bus, int n)
{
	(void)bus;
	(void)n;
	return NULL;
}

int pci_rebar_bytes_to_size(u64 bytes)
{
	bytes = roundup_pow_of_two(bytes);
	return max(ilog2(bytes), 20) - 20;
}

int pci_rebar_get_max_size(struct pci_dev *pdev, int bar)
{
	(void)pdev;
	(void)bar;
	return -ENOTSUPP;
}

int pci_resize_resource(struct pci_dev *dev, int i, int size, int exclude_bars)
{
	(void)dev;
	(void)i;
	(void)size;
	(void)exclude_bars;
	return -ENOTSUPP;
}

/*
 * GPU resets: RDNA 4 resets through its PSP (mode 1), not a function
 * reset, and Stela keeps no configuration state across one.
 */
int pci_reset_function(struct pci_dev *dev)
{
	(void)dev;
	return -ENOTTY;
}

struct pci_saved_state *pci_store_saved_state(struct pci_dev *dev)
{
	(void)dev;
	return NULL;
}

int pci_load_saved_state(struct pci_dev *dev, struct pci_saved_state *state)
{
	(void)dev;
	return state ? -EINVAL : 0;
}

void pci_restore_msi_state(struct pci_dev *dev)
{
	(void)dev;
}

/* Every address the card reaches is in its DMA mask under Stela. */
bool dma_addressing_limited(struct device *dev)
{
	(void)dev;
	return false;
}

/* Peer-to-peer mappings (another device's BAR): none under Stela. */
dma_addr_t dma_map_resource(struct device *dev, phys_addr_t phys_addr, size_t size,
			    enum dma_data_direction dir, unsigned long attrs)
{
	(void)dev;
	(void)phys_addr;
	(void)size;
	(void)dir;
	(void)attrs;
	return DMA_MAPPING_ERROR;
}

void dma_unmap_resource(struct device *dev, dma_addr_t addr, size_t size,
			enum dma_data_direction dir, unsigned long attrs)
{
	(void)dev;
	(void)addr;
	(void)size;
	(void)dir;
	(void)attrs;
}

/* Only gmc_v9's NUMA placement asks, which RDNA 4 does not run. */
int page_is_ram(unsigned long pfn)
{
	(void)pfn;
	return 0;
}

time64_t ktime_get_real_seconds(void)
{
	struct timespec64 now;

	ktime_get_real_ts64(&now);
	return now.tv_sec;
}

/*
 * Interrupt vectors: amdgpu asks for exactly one, MSI when it may. Stela
 * gives a function one MSI (pci_enable_msi), its IRQ in dev->irq, or its
 * legacy interrupt there.
 */
int pci_alloc_irq_vectors(struct pci_dev *dev, unsigned int min_vecs, unsigned int max_vecs,
			  unsigned int flags)
{
	if (min_vecs > 1 || max_vecs < 1)
		return -ENOSPC;
	if ((flags & PCI_IRQ_MSI) && !pci_enable_msi(dev))
		return 1;
	if (flags & PCI_IRQ_INTX)
		return 1;
	return -ENOSPC;
}

int pci_irq_vector(struct pci_dev *dev, unsigned int nr)
{
	return nr == 0 ? (int)dev->irq : -EINVAL;
}

void pci_free_irq_vectors(struct pci_dev *dev)
{
	pci_disable_msi(dev);
}
