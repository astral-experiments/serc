def f(flag):
 with manager():
  if flag:
   assert flag
  else:
   assert not flag
  yield 1
  raise StopIteration()
