def f(a,b,c,ys):
 try:
  for v in ys:
   if a:
    if b or c:
     consume(v)
    else:
     assert b
   else:
    consume(v)
  after = a
 except ValueError:
  recover()
