def f(a,b):
 with manager():
  if a and b:
   assert a
  else:
   assert b
  work()
