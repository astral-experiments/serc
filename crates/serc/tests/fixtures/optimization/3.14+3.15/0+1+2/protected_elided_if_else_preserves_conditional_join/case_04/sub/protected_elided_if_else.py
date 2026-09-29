def f(a, b):
 try:
  if a or b:
   assert a
  else:
   assert b
  work(a)
 except ValueError:
  recover()
