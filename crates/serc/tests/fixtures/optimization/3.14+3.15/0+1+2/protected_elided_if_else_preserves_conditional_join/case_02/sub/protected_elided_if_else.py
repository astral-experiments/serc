def f(a):
 try:
  if a is None:
   assert a
  else:
   assert not a
  work(a)
 except ValueError:
  recover()
 else:
  finish()
