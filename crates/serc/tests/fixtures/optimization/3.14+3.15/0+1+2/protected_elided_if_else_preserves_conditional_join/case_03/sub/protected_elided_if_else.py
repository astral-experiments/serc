def f(a, b):
 try:
  if a:
   assert a
  else:
   assert b
  assert b
 except ValueError:
  recover()
