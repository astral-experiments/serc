def f(a, b, x, ys):
 try:
  if a:
   assert a
  else:
   assert b
  x = 3
  if b:
   x = [i for i in ys]
  assert a
 except ValueError:
  work()
