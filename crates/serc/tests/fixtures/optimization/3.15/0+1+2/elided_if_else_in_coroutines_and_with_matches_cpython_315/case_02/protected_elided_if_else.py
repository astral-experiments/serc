def f(flag):
 with manager():
  if flag == 'v1':
   assert flag
  else:
   assert not flag
