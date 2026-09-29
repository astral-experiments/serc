def f(x):
 if x:
  with context():
   yield x
 else:
  if x:
   assert x
  assert x
