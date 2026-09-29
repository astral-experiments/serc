def enclosing():
 value = 1
 def f(xs, work, recover):
  try:
   work()
  except ValueError:
   recover()
  for x in xs:
   if x:
    assert False
   if x:
    assert False
  yield value
 return f
