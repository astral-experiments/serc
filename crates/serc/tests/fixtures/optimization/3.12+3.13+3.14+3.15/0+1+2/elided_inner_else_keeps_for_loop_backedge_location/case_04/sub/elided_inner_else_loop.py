def f(xs,a,b,c):
 for x in xs:
  if a:
   if b or c:
    z = x
   else:
    assert b
  else:
   other(x)
