def typed():
    try:
        body()
    except Error:
        recover()
        pass
    after()

def bare():
    try:
        body()
    except:
        recover()
        pass
    after()

def named():
    try:
        body()
    except Error as error:
        recover(error)
        pass
    after()
