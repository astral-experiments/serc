async def choose(value):
    return value or 'empty'

try:
    body()
except FirstError or SecondError as error:
    recover(error)

try:
    body()
except FirstError and SecondError as error:
    recover(error)
