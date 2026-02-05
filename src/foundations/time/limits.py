import sys

# We need to find the largest time value where the spacing between
# representable f64 values is still <= 0.01 seconds

# f64 has 53 bits of precision (52 explicit + 1 implicit)
# The spacing between adjacent f64 values at magnitude x is:
# spacing = x * 2^(-52) for normalized numbers

# We want: spacing <= 0.01
# So: x * 2^(-52) <= 0.01
# Therefore: x <= 0.01 * 2^52

max_time_seconds = 0.01 * (2 ** 52)

print(f"Maximum time with 0.01s precision: {max_time_seconds:.2f} seconds")
print()

# Let's verify this by checking the spacing at this value
next_value = max_time_seconds + max_time_seconds * sys.float_info.epsilon
spacing = next_value - max_time_seconds
print(f"Actual spacing at this value: {spacing:.6f} seconds")
print()

# Convert to years, days, hours, minutes, seconds
seconds_per_minute = 60.0
seconds_per_hour = 60.0 * seconds_per_minute
seconds_per_day = 24.0 * seconds_per_hour
seconds_per_year = 365.25 * seconds_per_day  # Account for leap years

remaining = max_time_seconds

years = int(remaining / seconds_per_year)
remaining -= years * seconds_per_year

days = int(remaining / seconds_per_day)
remaining -= days * seconds_per_day

hours = int(remaining / seconds_per_hour)
remaining -= hours * seconds_per_hour

minutes = int(remaining / seconds_per_minute)
remaining -= minutes * seconds_per_minute

seconds = remaining

print("Formatted result:")
print(f"{years} years, {days} days, {hours} hours, {minutes} minutes, {seconds:.2f} seconds")

# Also show in a more readable summary
print()
print("Summary: An f64 storing time in seconds can track approximately")
print(f"{years:,} years without losing more than 1/100th of a second precision")
