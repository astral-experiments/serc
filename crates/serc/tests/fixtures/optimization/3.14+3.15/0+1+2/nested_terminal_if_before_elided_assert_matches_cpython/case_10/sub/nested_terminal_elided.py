def f(a,b):
 global x
 if a:
  if b:
   work()
  else:
   assert (x := a)
  assert b
