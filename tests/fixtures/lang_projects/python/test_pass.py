import unittest
from proof import add

class Passing(unittest.TestCase):
    def test_add(self):
        self.assertEqual(add(1, 1), 2)
