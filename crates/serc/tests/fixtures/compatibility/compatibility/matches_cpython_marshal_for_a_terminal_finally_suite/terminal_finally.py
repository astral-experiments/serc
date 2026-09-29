"""doc"""
try:
    raise ValueError
finally:
    raise RuntimeError
after = 1
try:
    action()
except Error as captured:
    consume(captured)
