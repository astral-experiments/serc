async def f(x):
 match x:
  case str() | bytes():
   assert (z := x)
  case int():
   work()
  case _:
   other()
 await work()
