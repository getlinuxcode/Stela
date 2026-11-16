// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * Kernel services amdgpu reaches that Stela's Linux environment does not
 * define, and whose answer on Stela is plain (AMD fork, J6). Each says why
 * its answer holds for an RDNA 4 desktop card under Stela.
 */
#include <linux/backlight.h>
#include <linux/device.h>
#include <linux/err.h>
#include <linux/hwmon.h>
#include <linux/kstrtox.h>
#include <linux/printk.h>
#include <linux/sysfs.h>
#include <linux/timekeeping.h>

/*
 * Stela has no sysfs: nothing reads amdgpu's attribute files, so adding
 * them succeeds and removing them has nothing to undo.
 */
int device_create_file(struct device *dev, const struct device_attribute *attr)
{
	(void)dev;
	(void)attr;
	return 0;
}

void device_remove_file(struct device *dev, const struct device_attribute *attr)
{
	(void)dev;
	(void)attr;
}

int devm_device_add_group(struct device *dev, const struct attribute_group *grp)
{
	(void)dev;
	(void)grp;
	return 0;
}

/*
 * No hwmon class: amdgpu_pm_sysfs_init logs the refusal and the device
 * starts without its sensors' files.
 */
struct device *hwmon_device_register_with_groups(struct device *dev, const char *name,
						 void *drvdata,
						 const struct attribute_group **groups)
{
	(void)dev;
	(void)name;
	(void)drvdata;
	(void)groups;
	return ERR_PTR(-ENODEV);
}

void hwmon_device_unregister(struct device *dev)
{
	(void)dev;
}

/*
 * A backlight belongs to an internal panel (eDP), which a desktop card
 * does not drive; the display core goes on without one when refused.
 */
struct backlight_device *backlight_device_register(const char *name, struct device *dev,
						   void *devdata, const struct backlight_ops *ops,
						   const struct backlight_properties *props)
{
	(void)name;
	(void)dev;
	(void)devdata;
	(void)ops;
	(void)props;
	return ERR_PTR(-ENODEV);
}

void backlight_device_unregister(struct backlight_device *bd)
{
	(void)bd;
}

/* Rate limiting a message is the console's affair: every one is printed. */
int __printk_ratelimit(const char *func)
{
	(void)func;
	return 1;
}

/*
 * Only debugfs and sysfs writes hand amdgpu a user string, and Stela has
 * neither.
 */
int kstrtouint_from_user(const char __user *s, size_t count, unsigned int base,
			 unsigned int *res)
{
	(void)s;
	(void)count;
	(void)base;
	(void)res;
	return -EINVAL;
}

/* One clock: Stela's monotonic time is not slewed. */
ktime_t ktime_get_raw(void)
{
	return ktime_get();
}

u64 ktime_get_mono_fast_ns(void)
{
	return ktime_get_ns();
}

/*
 * The interrupt chip amdgpu's domain gives its child devices' interrupts;
 * Stela never makes that domain (stela/amdgpu.h), so nothing maps one.
 */
#include <linux/irq.h>

void handle_simple_irq(struct irq_desc *desc)
{
	(void)desc;
	WARN_ONCE(1, "amdgpu: no interrupt domain under Stela\n");
}

void irq_set_chip_and_handler_name(unsigned int irq, const struct irq_chip *chip,
				   irq_flow_handler_t handle, const char *name)
{
	(void)irq;
	(void)chip;
	(void)handle;
	(void)name;
	WARN_ONCE(1, "amdgpu: no interrupt domain under Stela\n");
}

/*
 * A bus amdgpu adds (a connector's DDC, the SMU's EEPROM bus) is deleted
 * with the device's other managed actions, as devres.c's devm_ variant.
 */
#include <linux/i2c.h>

static void delete_adapter(void *adapter)
{
	i2c_del_adapter(adapter);
}

int devm_i2c_add_adapter(struct device *dev, struct i2c_adapter *adapter)
{
	int error = i2c_add_adapter(adapter);

	if (error)
		return error;
	return devm_add_action_or_reset(dev, delete_adapter, adapter);
}

/*
 * Masking a requested line, nested as Linux's depth counts: amdgpu stops
 * its fence interrupt around a GPU reset (amdgpu_fence_driver_isr_toggle).
 * The owner masks the source (irq_disable); an owner runs its handlers on
 * its one service CPU, in atomic scopes this caller cannot be inside, so
 * none is running when the mask returns, which is what disable_irq waits
 * for. amdgpu's handler is a primary one, without a thread to wait for.
 */
#include <linux/interrupt.h>
#include <linux/mutex.h>
#include <linux/slab.h>
#include <stela/linux.h>

struct masked_line {
	struct list_head node;
	unsigned int irq;
	unsigned int depth;
};

static LIST_HEAD(masked_lines);
static DEFINE_MUTEX(masked_mutex);

static struct masked_line *masked_line(unsigned int irq)
{
	struct masked_line *line;

	list_for_each_entry(line, &masked_lines, node)
		if (line->irq == irq)
			return line;
	return NULL;
}

void disable_irq(unsigned int irq)
{
	const struct stela_linux_owner *owner = stela_linux_owner();
	struct masked_line *line;

	mutex_lock(&masked_mutex);
	line = masked_line(irq);
	if (!line) {
		line = kzalloc_obj(*line);
		if (!line)
			stela_linux_fatal("amdgpu: no memory to mask an interrupt line");
		line->irq = irq;
		list_add(&line->node, &masked_lines);
	}
	if (line->depth++ == 0 &&
	    (!owner->services->irq_disable || owner->services->irq_disable(owner->context, irq)))
		stela_linux_fatal("amdgpu: interrupt mask unavailable");
	mutex_unlock(&masked_mutex);
}

void enable_irq(unsigned int irq)
{
	const struct stela_linux_owner *owner = stela_linux_owner();
	struct masked_line *line;
	bool unexposed = false;

	mutex_lock(&masked_mutex);
	line = masked_line(irq);
	if (WARN(!line || !line->depth, "Unbalanced enable for IRQ %u\n", irq)) {
		mutex_unlock(&masked_mutex);
		return;
	}
	if (--line->depth == 0) {
		list_del(&line->node);
		kfree(line);
		if (!owner->services->irq_enable ||
		    owner->services->irq_enable(owner->context, irq, &unexposed))
			stela_linux_fatal("amdgpu: interrupt unmask unavailable");
	}
	mutex_unlock(&masked_mutex);
}

/*
 * A fatal uncorrectable hardware error (RAS) with amdgpu's emergency
 * restart policy set: Stela stops at the owner's terminal boundary, which
 * logs it, rather than resetting the machine under its other services.
 */
#include <linux/reboot.h>

void emergency_restart(void)
{
	stela_linux_fatal("amdgpu: emergency restart after a fatal hardware error");
}

/*
 * The compute partition devices amdxcp makes, which only partitioned
 * Instinct GPUs (GFX 9.4.3 and later) ask for: none on a desktop card.
 */
#include <linux/platform_device.h>

struct platform_device *platform_device_register_full(const struct platform_device_info *info)
{
	(void)info;
	return ERR_PTR(-ENODEV);
}

/*
 * The environment never makes a device link (device_link_add is
 * unsupported); only an APU's USB power dependency would hold one.
 */
void device_link_del(struct device_link *link)
{
	WARN_ON(link);
}
