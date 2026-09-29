async def f(x):
 await work()
 match x:
  case str():
   assert x
  case int():
   work()
  case _:
   other()
 after()
