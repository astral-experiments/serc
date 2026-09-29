def f(flag):
 with manager():
  if flag:
   assert flag
  else:
   assert not flag
  work()
