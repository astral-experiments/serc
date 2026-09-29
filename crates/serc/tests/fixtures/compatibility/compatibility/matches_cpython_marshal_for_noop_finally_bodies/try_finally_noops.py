try:
    body()
except Error:
    recover()
finally:
    ...

try:
    body()
except Error:
    recover()
finally:
    pass
    pass

try:
    try:
        body()
    finally:
        pass
except Error:
    recover()
