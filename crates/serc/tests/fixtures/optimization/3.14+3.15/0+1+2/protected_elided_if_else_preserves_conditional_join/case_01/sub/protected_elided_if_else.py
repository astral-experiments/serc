def f(a):
 try:
  if a:
   assert a
  else:
   assert not a
  work(a)
 except ValueError:
  recover()
