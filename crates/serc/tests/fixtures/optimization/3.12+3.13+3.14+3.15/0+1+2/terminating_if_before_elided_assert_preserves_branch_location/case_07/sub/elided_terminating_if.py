def f(a,b,c):
 if a:
  if True:
   raise ValueError()
  assert a
 else:
  other()
 after()
