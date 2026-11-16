// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * sscanf, which Stela's Linux environment does not define (AMD fork, J6),
 * with the conversions Linux's lib/vsprintf.c vsscanf accepts: whitespace
 * matches any run of it, %% and other characters match themselves, and
 * %[*][width][hh|h|l|ll|L|z|j|t]{d,i,u,o,x,X,s,c,n} convert. amdgpu parses
 * its disable_cu parameter ("%u.%u.%u") and its debugfs and sysfs writes
 * with it. As Linux's, it stops at the first conversion that does not
 * match and returns the number assigned.
 */
#include <linux/ctype.h>
#include <linux/kernel.h>
#include <linux/limits.h>
#include <linux/stdarg.h>
#include <linux/string.h>

enum size { SIZE_CHAR, SIZE_SHORT, SIZE_INT, SIZE_LONG, SIZE_LONG_LONG, SIZE_SIZE };

static unsigned int digit_value(char c)
{
	if (c >= '0' && c <= '9')
		return c - '0';
	c = tolower(c);
	if (c >= 'a' && c <= 'f')
		return c - 'a' + 10;
	return 16;
}

/*
 * An integer of at most width characters in base (0 detects 0x and 0),
 * at *text; false when no digit is there.
 */
static bool scan_number(const char **text, unsigned int width, unsigned int base, bool is_signed,
			unsigned long long *value)
{
	const char *s = *text;
	bool negative = false;
	unsigned long long result = 0;
	const char *digits;

	if (is_signed && width && (*s == '-' || *s == '+')) {
		negative = *s == '-';
		s++;
		width--;
	}
	if ((base == 0 || base == 16) && width >= 3 && s[0] == '0' && tolower(s[1]) == 'x' &&
	    isxdigit(s[2])) {
		s += 2;
		width -= 2;
		base = 16;
	} else if (base == 0) {
		base = s[0] == '0' ? 8 : 10;
	}
	digits = s;
	while (width && digit_value(*s) < base) {
		result = result * base + digit_value(*s);
		s++;
		width--;
	}
	if (s == digits)
		return false;
	*text = s;
	*value = negative ? -result : result;
	return true;
}

static void store(va_list *args, enum size size, unsigned long long value)
{
	switch (size) {
	case SIZE_CHAR:
		*va_arg(*args, unsigned char *) = value;
		break;
	case SIZE_SHORT:
		*va_arg(*args, unsigned short *) = value;
		break;
	case SIZE_INT:
		*va_arg(*args, unsigned int *) = value;
		break;
	case SIZE_LONG:
		*va_arg(*args, unsigned long *) = value;
		break;
	case SIZE_LONG_LONG:
		*va_arg(*args, unsigned long long *) = value;
		break;
	case SIZE_SIZE:
		*va_arg(*args, size_t *) = value;
		break;
	}
}

int vsscanf(const char *buf, const char *fmt, va_list args)
{
	const char *s = buf;
	int assigned = 0;
	va_list ap;

	va_copy(ap, args);
	while (*fmt) {
		if (isspace(*fmt)) {
			fmt = skip_spaces(fmt);
			s = skip_spaces(s);
			continue;
		}
		if (*fmt != '%' || fmt[1] == '%') {
			fmt += *fmt == '%';
			if (*fmt++ != *s++)
				break;
			continue;
		}
		fmt++;

		bool skip = *fmt == '*';
		unsigned int width = UINT_MAX;
		enum size size = SIZE_INT;

		fmt += skip;
		if (isdigit(*fmt)) {
			width = 0;
			while (isdigit(*fmt))
				width = width * 10 + (*fmt++ - '0');
			if (!width)
				break;
		}
		if (*fmt == 'h') {
			size = fmt[1] == 'h' ? SIZE_CHAR : SIZE_SHORT;
			fmt += size == SIZE_CHAR ? 2 : 1;
		} else if (*fmt == 'l') {
			size = fmt[1] == 'l' ? SIZE_LONG_LONG : SIZE_LONG;
			fmt += size == SIZE_LONG_LONG ? 2 : 1;
		} else if (*fmt == 'L') {
			size = SIZE_LONG_LONG;
			fmt++;
		} else if (*fmt == 'z' || *fmt == 'j' || *fmt == 't') {
			size = *fmt == 'z' ? SIZE_SIZE : SIZE_LONG_LONG;
			fmt++;
		}

		char conversion = *fmt++;
		unsigned int base = 10;
		bool is_signed = false;

		if (conversion == 'n') {
			if (!skip)
				*va_arg(ap, int *) = s - buf;
			continue;
		}
		if (conversion == 'c') {
			char *out = skip ? NULL : va_arg(ap, char *);

			if (width == UINT_MAX)
				width = 1;
			for (; width; width--, s++) {
				if (!*s)
					goto done;
				if (out)
					*out++ = *s;
			}
			assigned += !skip;
			continue;
		}
		s = skip_spaces(s);
		if (!*s)
			break;
		if (conversion == 's') {
			char *out = skip ? NULL : va_arg(ap, char *);

			for (; width && *s && !isspace(*s); width--, s++)
				if (out)
					*out++ = *s;
			if (out)
				*out = '\0';
			assigned += !skip;
			continue;
		}
		switch (conversion) {
		case 'd':
			is_signed = true;
			break;
		case 'i':
			is_signed = true;
			base = 0;
			break;
		case 'u':
			break;
		case 'o':
			base = 8;
			break;
		case 'x':
		case 'X':
			base = 16;
			break;
		default:
			goto done;
		}

		unsigned long long value;

		if (!scan_number(&s, width, base, is_signed, &value))
			break;
		if (!skip) {
			store(&ap, size, value);
			assigned++;
		}
	}
done:
	va_end(ap);
	return assigned;
}

int sscanf(const char *buf, const char *fmt, ...)
{
	va_list args;
	int assigned;

	va_start(args, fmt);
	assigned = vsscanf(buf, fmt, args);
	va_end(args);
	return assigned;
}
