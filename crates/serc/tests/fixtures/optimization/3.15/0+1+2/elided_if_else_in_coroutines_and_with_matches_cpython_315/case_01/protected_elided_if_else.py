async def f(flag):
 if flag is not False:
  assert flag
 else:
  assert not flag
 await work()
