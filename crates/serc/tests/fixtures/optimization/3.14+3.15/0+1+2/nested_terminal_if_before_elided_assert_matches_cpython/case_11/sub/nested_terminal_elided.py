def outer():
 x=0
 def f(a,b):
  nonlocal x
  if a:
   if b:
    work()
   else:
    assert (x := a)
   assert b
 return f
