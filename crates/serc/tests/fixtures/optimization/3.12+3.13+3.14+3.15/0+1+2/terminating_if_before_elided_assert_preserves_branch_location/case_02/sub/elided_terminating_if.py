def f(a,b,c):
 if a:
  if not b:
   raise ValueError()
  assert a
 else:
  other()
 after()
