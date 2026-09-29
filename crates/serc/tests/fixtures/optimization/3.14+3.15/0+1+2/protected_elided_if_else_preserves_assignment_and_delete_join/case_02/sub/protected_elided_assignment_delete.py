def f(a):
 try:
  if a:
   assert a
  else:
   assert not a
  del a
 except ValueError:
  recover()
