def f(x):
 match x:
  case A(foo=y) | B(foo=y):
   assert x
  case int():
   work()
  case _:
   other()
 after()
