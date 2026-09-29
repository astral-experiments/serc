def f(xs, work, cleanup):
 try:
  work()
 finally:
  cleanup()
 for x in xs:
  if x:
   assert False
  if x:
   assert False
