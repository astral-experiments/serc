async def f(x):
 yield x
 match x:
  case str() | bytes():
   assert x
  case int():
   work()
  case _:
   other()
 after()
