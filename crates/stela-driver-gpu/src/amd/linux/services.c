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
