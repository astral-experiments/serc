async def f(flag):
 async with manager():
  if flag:
   assert flag
  else:
   assert not flag
  await work()
