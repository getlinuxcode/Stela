// SPDX-License-Identifier: GPL-2.0-or-later
/* Linux's I2C core on the fixture's real Runtime task, which its bus lock
 * needs: adapters, transfers with Linux's retries, and unbound clients. */
#include <linux/err.h>
#include <linux/i2c.h>
#include <linux/jiffies.h>
#include <linux/string.h>
#include <stela/test.h>

void stela_linux_i2c_test(void);

static int results[2];
static unsigned int transfers;

static int transfer(struct i2c_adapter *adapter, struct i2c_msg *msgs, int num)
{
	(void)adapter;
	(void)msgs;
	(void)num;
	return results[transfers++];
}

static u32 functionality(struct i2c_adapter *adapter)
{
	(void)adapter;
	return I2C_FUNC_I2C;
}

static const struct i2c_algorithm algorithm = {.xfer = transfer, .functionality = functionality};

static struct i2c_adapter adapter = {.algo = &algorithm, .retries = 1, .name = "test"};

/* amdgpu's SMU bus: a write then a read to one address, short messages. */
static const struct i2c_adapter_quirks quirks = {
	.flags = I2C_AQ_COMB | I2C_AQ_COMB_SAME_ADDR | I2C_AQ_NO_ZERO_LEN,
	.max_read_len = 8,
	.max_write_len = 8,
	.max_comb_1st_msg_len = 2,
	.max_comb_2nd_msg_len = 6,
};
static struct i2c_adapter limited = {.algo = &algorithm, .quirks = &quirks, .name = "limited"};

/* The core refuses what an adapter's quirks rule out before its transfer. */
static void quirks_test(void)
{
	u8 bytes[10] = {};
	struct i2c_msg msgs[3] = {
		{.addr = 0x50, .len = 2, .buf = bytes},
		{.addr = 0x50, .flags = I2C_M_RD, .len = 6, .buf = bytes},
		{.addr = 0x50, .len = 1, .buf = bytes},
	};

	assert(!i2c_add_adapter(&limited));
	transfers = 0;
	results[0] = 2;
	assert(i2c_transfer(&limited, msgs, 2) == 2 && transfers == 1);
	transfers = 0;
	msgs[1].addr = 0x51;
	assert(i2c_transfer(&limited, msgs, 2) == -EOPNOTSUPP);
	msgs[1].addr = 0x50;
	msgs[1].len = 7;
	assert(i2c_transfer(&limited, msgs, 2) == -EOPNOTSUPP);
	assert(i2c_transfer(&limited, msgs, 3) == -EOPNOTSUPP);
	msgs[2].len = 0;
	assert(i2c_transfer(&limited, &msgs[2], 1) == -EOPNOTSUPP);
	msgs[2].len = 9;
	assert(i2c_transfer(&limited, &msgs[2], 1) == -EOPNOTSUPP && !transfers);
	i2c_del_adapter(&limited);
}

void stela_linux_i2c_test(void)
{
	assert(!i2c_add_adapter(&adapter) && adapter.timeout == HZ);
	assert(!strcmp(dev_name(&adapter.dev), "i2c-0"));

	/* Arbitration loss retries up to the limit; other errors do not. */
	results[0] = -EAGAIN;
	results[1] = 2;
	assert(i2c_transfer(&adapter, NULL, 2) == 2 && transfers == 2);
	transfers = 0;
	results[0] = results[1] = -EAGAIN;
	assert(i2c_transfer(&adapter, NULL, 1) == -EAGAIN && transfers == 2);
	transfers = 0;
	results[0] = -EIO;
	assert(i2c_transfer(&adapter, NULL, 1) == -EIO && transfers == 1);

	assert(i2c_trylock_bus(&adapter, I2C_LOCK_SEGMENT));
	assert(!i2c_trylock_bus(&adapter, I2C_LOCK_SEGMENT));
	i2c_unlock_bus(&adapter, I2C_LOCK_SEGMENT);

	struct i2c_board_info info = {I2C_BOARD_INFO("sensor", 0x48)};
	struct i2c_client *client = i2c_new_client_device(&adapter, &info);
	assert(!IS_ERR(client) && client->addr == 0x48 && !client->dev.driver);
	assert(!strcmp(client->name, "sensor") && client->adapter == &adapter);
	i2c_unregister_device(client);
	i2c_unregister_device(ERR_PTR(-ENODEV));
	info.addr = 0x80;
	assert(IS_ERR(i2c_new_client_device(&adapter, &info)));

	i2c_del_adapter(&adapter);
	quirks_test();
}
