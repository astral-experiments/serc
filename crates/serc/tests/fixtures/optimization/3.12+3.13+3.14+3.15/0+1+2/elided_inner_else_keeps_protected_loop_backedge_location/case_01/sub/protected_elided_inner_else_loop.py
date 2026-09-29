def f(a,b,c,ys):
 try:
  for v in ys:
   if a:
    if b:
     z = v
    else:
     assert b
   else:
    consume(v)
 except ValueError:
  recover()
