def f(x):
 match x:
  case str():
   assert x
   work()
  case int():
   work()
  case _:
   other()
 after()
