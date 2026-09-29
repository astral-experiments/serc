def f(xs, x):
 for item in xs:
  if x:
   assert False
  if x:
   assert False
 try:
  z = x + 1
  pass
 except ValueError:
  recover()
