def f(xs, cleanup):
 try:
  for x in xs:
   if x:
    assert False
   if x:
    assert False
 finally:
  cleanup()
