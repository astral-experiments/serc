def f(xs,a,b):
 global z
 for x in xs:
  if a:
   if b:
    z = x
   else:
    assert b
  else:
   other(x)
