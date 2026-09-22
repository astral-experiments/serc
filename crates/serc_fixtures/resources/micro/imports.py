import package
import package.module
import package.module as renamed
from package import first, second
from package.module import value as imported_value
from . import sibling
from ..parent import child
from package.exports import *


def local_imports():
    import local
    from local import value
    return local, value
