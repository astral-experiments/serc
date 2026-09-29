def f(a,b,c,ys):
 try:
  for v in ys:
   if a:
    if not b:
     consume(v)
    else:
     assert b
   else:
    consume(v)
  z = a + 1
  pass
  pass
 except ValueError:
  recover()
