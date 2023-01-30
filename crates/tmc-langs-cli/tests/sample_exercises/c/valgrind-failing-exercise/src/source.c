#include <stdio.h>
#include "source.h"

int one(void)
{
  return 1;
}

int two(void)
{
  int *i;
  i = (int*) malloc(8*sizeof(int));
  return 2;
}
