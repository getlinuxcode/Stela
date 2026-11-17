// SPDX-License-Identifier: GPL-2.0-or-later
/* Parts are adapted from the Linux files named below, whose notices are:
 * Copyright (C) 1995-99 Simon G. Vogl
 * Copyright (C) 2013-2017 Wolfram Sang <wsa@kernel.org>
 */
/* Linux's I2C core: adapters, transfers and client devices, following Linux
 * cc9de5d4b5ab6459f5774946aa3195ad51236fa8 drivers/i2c/i2c-core-base.c. No
 * I2C drivers run here, so a client stays unbound, which is what a Linux
 * caller sees when no driver claims a device. */
#include <linux/completion.h>
#include <linux/err.h>
#include <linux/i2c.h>
#include <linux/jiffies.h>
#include <linux/rtmutex.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <stela/linux.h>

static int next_adapter;

static void lock_bus(struct i2c_adapter *adapter, unsigned int flags)
{
	(void)flags;
	rt_mutex_lock(&adapter->bus_lock);
}

static int trylock_bus(struct i2c_adapter *adapter, unsigned int flags)
{
	(void)flags;
	return rt_mutex_trylock(&adapter->bus_lock);
}

static void unlock_bus(struct i2c_adapter *adapter, unsigned int flags)
{
	(void)flags;
	rt_mutex_unlock(&adapter->bus_lock);
}

static const struct i2c_lock_operations bus_lock_ops = {
	.lock_bus = lock_bus,
	.trylock_bus = trylock_bus,
	.unlock_bus = unlock_bus,
};

static void adapter_released(struct device *dev)
{
	complete(&to_i2c_adapter(dev)->dev_released);
}

/* Adapter numbers name no fixed board wiring under Stela. */
int i2c_add_numbered_adapter(struct i2c_adapter *adapter)
{
	return i2c_add_adapter(adapter);
}

int i2c_add_adapter(struct i2c_adapter *adapter)
{
	if (WARN(!adapter->name[0], "an I2C adapter has no name\n"))
		return -EINVAL;
	if (WARN(!adapter->algo, "adapter '%s': no algo supplied\n", adapter->name))
		return -EINVAL;
	if (!adapter->lock_ops)
		adapter->lock_ops = &bus_lock_ops;
	rt_mutex_init(&adapter->bus_lock);
	rt_mutex_init(&adapter->mux_lock);
	if (!adapter->timeout)
		adapter->timeout = HZ;
	init_completion(&adapter->dev_released);
	adapter->nr = next_adapter++;
	device_initialize(&adapter->dev);
	adapter->dev.release = adapter_released;
	return dev_set_name(&adapter->dev, "i2c-%d", adapter->nr);
}

void i2c_del_adapter(struct i2c_adapter *adapter)
{
	put_device(&adapter->dev);
	wait_for_completion(&adapter->dev_released);
}

static bool quirk_exceeded(int value, int max)
{
	return max && value > max;
}

static int quirk_error(struct i2c_adapter *adapter, const struct i2c_msg *msg, const char *why)
{
	dev_err_ratelimited(&adapter->dev, "adapter quirk: %s (addr 0x%04x, size %u, %s)\n", why,
			    msg->addr, msg->len, msg->flags & I2C_M_RD ? "read" : "write");
	return -EOPNOTSUPP;
}

/* The messages an adapter's controller cannot carry, as Linux's
 * i2c_check_for_quirks refuses them before the transfer. */
static int check_quirks(struct i2c_adapter *adapter, struct i2c_msg *msgs, int num)
{
	const struct i2c_adapter_quirks *q = adapter->quirks;
	int max_num = q->max_num_msgs;
	bool check_lengths = true;

	if (q->flags & I2C_AQ_COMB) {
		max_num = 2;
		if (num == 2) {
			if (q->flags & I2C_AQ_COMB_WRITE_FIRST && msgs[0].flags & I2C_M_RD)
				return quirk_error(adapter, &msgs[0], "1st comb msg must be write");
			if (q->flags & I2C_AQ_COMB_READ_SECOND && !(msgs[1].flags & I2C_M_RD))
				return quirk_error(adapter, &msgs[1], "2nd comb msg must be read");
			if (q->flags & I2C_AQ_COMB_SAME_ADDR && msgs[0].addr != msgs[1].addr)
				return quirk_error(adapter, &msgs[0], "comb msg only to same addr");
			if (quirk_exceeded(msgs[0].len, q->max_comb_1st_msg_len))
				return quirk_error(adapter, &msgs[0], "msg too long");
			if (quirk_exceeded(msgs[1].len, q->max_comb_2nd_msg_len))
				return quirk_error(adapter, &msgs[1], "msg too long");
			check_lengths = false;
		}
	}
	if (quirk_exceeded(num, max_num))
		return quirk_error(adapter, &msgs[0], "too many messages");
	for (int i = 0; i < num; i++) {
		u16 len = msgs[i].len;

		if (msgs[i].flags & I2C_M_RD) {
			if (check_lengths && quirk_exceeded(len, q->max_read_len))
				return quirk_error(adapter, &msgs[i], "msg too long");
			if (q->flags & I2C_AQ_NO_ZERO_LEN_READ && len == 0)
				return quirk_error(adapter, &msgs[i], "no zero length");
		} else {
			if (check_lengths && quirk_exceeded(len, q->max_write_len))
				return quirk_error(adapter, &msgs[i], "msg too long");
			if (q->flags & I2C_AQ_NO_ZERO_LEN_WRITE && len == 0)
				return quirk_error(adapter, &msgs[i], "no zero length");
		}
	}
	return 0;
}

/* Arbitration loss retries, within the adapter's limits. */
int __i2c_transfer(struct i2c_adapter *adapter, struct i2c_msg *msgs, int num)
{
	unsigned long start = jiffies;
	int ret = -EOPNOTSUPP;

	if (!adapter->algo->xfer)
		return ret;
	if (adapter->quirks && check_quirks(adapter, msgs, num))
		return -EOPNOTSUPP;
	for (int attempt = 0; attempt <= adapter->retries; attempt++) {
		ret = adapter->algo->xfer(adapter, msgs, num);
		if (ret != -EAGAIN || time_after(jiffies, start + (unsigned long)adapter->timeout))
			break;
	}
	return ret;
}

int i2c_transfer(struct i2c_adapter *adapter, struct i2c_msg *msgs, int num)
{
	i2c_lock_bus(adapter, I2C_LOCK_SEGMENT);
	int ret = __i2c_transfer(adapter, msgs, num);
	i2c_unlock_bus(adapter, I2C_LOCK_SEGMENT);
	return ret;
}

static void client_released(struct device *dev)
{
	kfree(to_i2c_client(dev));
}

struct i2c_client *i2c_new_client_device(struct i2c_adapter *adapter,
					 const struct i2c_board_info *info)
{
	unsigned short limit = info->flags & I2C_CLIENT_TEN ? 0x3ff : 0x7f;

	if (!info->addr || info->addr > limit)
		return ERR_PTR(-EINVAL);
	struct i2c_client *client = kzalloc_obj(*client);
	if (!client)
		return ERR_PTR(-ENOMEM);
	client->adapter = adapter;
	client->flags = info->flags;
	client->addr = info->addr;
	client->init_irq = info->irq;
	client->irq = info->irq;
	strscpy(client->name, info->type, sizeof(client->name));
	device_initialize(&client->dev);
	client->dev.parent = &adapter->dev;
	client->dev.platform_data = info->platform_data;
	client->dev.release = client_released;
	return client;
}

void i2c_unregister_device(struct i2c_client *client)
{
	if (!IS_ERR_OR_NULL(client))
		put_device(&client->dev);
}
