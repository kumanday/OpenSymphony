import pathlib
from collections import deque


class Worker:
    def run(self):
        return pathlib.Path(".").resolve()


def make_queue():
    queue = deque()
    queue.append("ready")
    return queue


def test_make_queue():
    assert make_queue()
