def f(x):
 match x:
  case None:
   pass
  case str() | bytes():
   assert x
   assert (w := x)
  case int():
   work()
  case _:
   other()
 after()
