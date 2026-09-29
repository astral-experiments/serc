def f(a, ys):
 try:
  x = 3
  if a:
   x = {i for i in ys}
  assert a
 except ValueError:
  work()
