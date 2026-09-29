def f(x):
 with context():
  if x:
   with context():
    call()
  else:
   if x:
    assert x
   assert x
