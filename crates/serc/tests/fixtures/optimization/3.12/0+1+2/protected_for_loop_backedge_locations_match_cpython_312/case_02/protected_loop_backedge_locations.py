def f(xs, manager):
 with manager:
  for x in xs:
   if x:
    assert False
   if x:
    assert False
