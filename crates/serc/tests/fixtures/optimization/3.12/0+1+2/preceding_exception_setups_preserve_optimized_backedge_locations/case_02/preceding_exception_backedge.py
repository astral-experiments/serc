def f(xs, work, recover):
 try:
  try:
   work()
  except ValueError:
   recover()
 except Exception:
  recover()
 for x in xs:
  if not x:
   assert False
  if not x:
   assert False
