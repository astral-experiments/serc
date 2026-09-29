def f(x):
 if x:
  call()
 else:
  with context():
   call()
  if x:
   assert x
  assert x
