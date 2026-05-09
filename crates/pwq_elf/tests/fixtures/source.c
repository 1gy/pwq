int win(void)    { return 0xdead; }
int target(void) { return 0xbeef; }
int main(void)   { return win() + target(); }
