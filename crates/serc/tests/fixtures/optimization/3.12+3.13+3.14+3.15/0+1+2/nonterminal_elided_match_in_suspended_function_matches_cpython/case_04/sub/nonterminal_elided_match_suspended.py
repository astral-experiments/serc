def f(x):
 match x:
  case str(v) | bytes(v):
   assert (z := x)
  case int():
   work()
  case _:
   other()
 yield x
