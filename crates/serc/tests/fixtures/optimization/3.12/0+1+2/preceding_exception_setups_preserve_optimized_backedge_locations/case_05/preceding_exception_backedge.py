def f(xs, work, recover, y):
 try:
  work()
 except ValueError:
  recover()
 for x in xs:
  if x and y:
   assert False
  if x and y:
   assert False
  work()
