def f(a):
 try:
  if a:
   assert a
  else:
   assert not a
  a = a + 1
 except ValueError:
  recover()
