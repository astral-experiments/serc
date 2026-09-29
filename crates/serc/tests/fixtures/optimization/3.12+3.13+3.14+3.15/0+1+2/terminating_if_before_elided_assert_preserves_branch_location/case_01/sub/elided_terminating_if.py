def f(a,b,c):
 if a:
  if b:
   raise ValueError()
  assert a
 else:
  other()
 after()
