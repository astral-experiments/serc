def f(a,b,c):
 if a:
  if b or c:
   raise ValueError()
  assert a
 else:
  other()
 after()
