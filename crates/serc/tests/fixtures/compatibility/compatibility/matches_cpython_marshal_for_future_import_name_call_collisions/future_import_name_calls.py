from __future__ import annotations, generator_stop as stopped

def feature_name(annotations):
    return annotations.items()

def aliased_feature_name(stopped):
    return stopped.items()

def unrelated_name(receiver):
    return receiver.items()
