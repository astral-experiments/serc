def f(x):
 try:
  if x:
   with context():
    call()
  else:
   if x:
    assert x
   assert x
 except Exception:
  recover()
