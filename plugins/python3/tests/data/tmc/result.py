from tmc.hmac_writer import write_hmac
from unittest.runner import TextTestResult
from .points import _parse_points, _name_test
from copy import deepcopy
import atexit
import json
import traceback

module_secret = None
# Copy of the results for the Python editor
results = []

class TMCResult(TextTestResult):

    def __init__(self, stream, descriptions, verbosity):
        self.__results = []
        global module_secret
        secret = module_secret
        del module_secret
        that = self

        def write_output():
            nonlocal that
            nonlocal secret
            output = json.dumps(that.__results)
            if secret is not None:
                write_hmac(secret, output)
            with open(".tmc_test_results.json", "w") as text_file:
                text_file.write(output)
        atexit.register(write_output)
        super(TMCResult, self).__init__(stream, descriptions, verbosity)

    def startTest(self, test):
        super(TMCResult, self).startTest(test)

    def addSuccess(self, test):
        super(TMCResult, self).addSuccess(test)
        self.addResult(test, 'passed')

    def addFailure(self, test, err):
        super(TMCResult, self).addFailure(test, err)
        self.addResult(test, 'failed', err)

    def addError(self, test, err):
        super(TMCResult, self).addError(test, err)
        self.addResult(test, 'errored', err)

    def addResult(self, test, status, err=None):
        global results
        points = _parse_points(test)
        message = ""
        backtrace = []
        if err is not None:
            message = str(err[1])
            backtrace = traceback.format_tb(err[2])

        details = {
            'name': _name_test(test),
            'status': status,
            'message': message,
            'passed': status == 'passed',
            'points': points,
            'backtrace': backtrace
        }
        self.__results.append(details)
        results = deepcopy(self.__results)
