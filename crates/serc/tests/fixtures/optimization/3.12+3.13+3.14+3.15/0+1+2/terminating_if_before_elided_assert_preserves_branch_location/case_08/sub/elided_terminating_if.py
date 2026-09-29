def f(a,b,c,cm,xs):
 if a:
  try:
   work()
  finally:
   cleanup()
  if b:
   raise ValueError()
  assert a
 else:
  other()
 after()
