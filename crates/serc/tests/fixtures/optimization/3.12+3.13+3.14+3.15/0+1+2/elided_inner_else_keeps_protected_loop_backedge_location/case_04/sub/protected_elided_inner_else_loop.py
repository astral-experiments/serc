def f(a,b,c,ys):
 try:
  for v in ys:
   if a:
    if b < c:
     z = v
    else:
     assert b
   else:
    consume(v)
  after = a
 except ValueError:
  recover()
 else:
  consume(0)
