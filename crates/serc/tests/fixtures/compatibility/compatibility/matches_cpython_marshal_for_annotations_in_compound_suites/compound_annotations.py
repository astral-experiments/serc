with manager:
    in_with: WithAnnotation = 1

try:
    in_try: TryAnnotation = 2
except Error:
    in_except: ExceptAnnotation = 3
else:
    in_else: ElseAnnotation = 4
finally:
    in_finally: FinallyAnnotation = 5

match subject:
    case 1:
        in_match: MatchAnnotation = 6
