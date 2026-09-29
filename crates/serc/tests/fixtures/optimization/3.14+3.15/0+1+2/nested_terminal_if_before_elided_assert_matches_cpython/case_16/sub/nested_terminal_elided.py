def outer():
 x=0
 def f(a,b):
  nonlocal x
  if a:
   if b:
    x = 1
   assert a
 return f
