#include <string.h>
#include <stdio.h>

// Sometimes Netbeans TMC plugin pukes when it gets non-ASCII characters
// in test output. Trying to avoid that
void remove_nonascii(char *str)
{
    while (*str) {
        if (*str & 0x80)
            *str = '?';
        str++;
    }
}


void printchar(char *buf, char c)
{
    if (c == '\n') {
        strcpy(buf, "\\n");
    } else if (c & 0x80) {
        strcpy(buf, "(invalid)");
    } else {
        *buf++ = c;
        *buf = 0;
    }
}

int mycompare(char *student, char *model, char *infostr)
{
    int i = 0, line = 1;
    char bufS[16] = { 0 };
    char bufM[16] = { 0 };
    while(*student) {
        printchar(bufS, *student);
        printchar(bufM, *model);
        if (!(*model)) {
            sprintf(infostr, "your output is longer than expected: character: '%s', position: %d, line: %d\n",
                    bufS, i+1, line);
            return -1;
        }
        if (*student != *model) {
            sprintf(infostr, "position: %d, line: %d, your output: '%s' , expected: '%s'\n", 
                    i+1, line, bufS, bufM);
            return -1;
        }
        if (*student == '\n') {
            line++;
            i = -1;
        }
        student++; model++; i++;
    }
    if (*model) {
        printchar(bufM, *model);
        sprintf(infostr, "output correct until position: %d, line: %d, but shorter than expected. Next character should be '%s'\n",
                i+1, line, bufM);
        return -1;
    }
    return 0;
}
