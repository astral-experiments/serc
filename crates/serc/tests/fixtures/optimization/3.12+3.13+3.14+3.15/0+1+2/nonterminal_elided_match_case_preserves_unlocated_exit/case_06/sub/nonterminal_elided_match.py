def f(x):
 global z
 match x:
  case str() | bytes():
   assert (z := x)
  case int():
   work()
  case _:
   other()
 after()
