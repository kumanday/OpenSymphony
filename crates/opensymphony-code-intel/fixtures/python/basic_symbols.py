import pathlib
from collections import deque


class Worker:
    def run(self):
        def nested():
            return pathlib.Path(".").resolve()

        return nested()


def make_queue():
    queue = deque()
    queue.append("ready")
    return queue


def test_make_queue():
    assert make_queue()
