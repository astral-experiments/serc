def f(xs, work, recover):
 value = 1
 def g():
  return value
 try:
  work()
 except ValueError:
  recover()
 for x in xs:
  if x:
   assert False
  if x:
   assert False
 yield g
