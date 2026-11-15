.. csv-table:: Frozen *Delights*
   :header: "Treat", "Quantity", "Description"
   :widths: 15, 10, 30
   :align: right

   "Albatross", 2.99, "On a stick!"
   "Crunchy Frog", 1.49, "If we took the bones out, it wouldn't be
   crunchy, now would it?"
   "Gannet Ripple", 1.99, "On a stick!"

.. csv-table::
   :header-rows: 1
   :stub-columns: 1
   :delim: ;
   :keepspace:

   a; b; c
   1; 2
   x;y;z

.. csv-table::
   :quote: '
   :escape: \

   'a, b', c\,d

.. csv-table::
   :delim: tab

   a	b

.. csv-table::

   "unclosed

.. csv-table::

   "a"b

.. csv-table::
   :file: data.csv

.. csv-table::

.. csv-table::
   :widths: 1 2

   a, b, c

.. csv-table::
   :delim: ab

   x

.. csv-table::
   :header-rows: 3

   a
   b

.. csv-table::

   - *Cell* with markup, ``literal``

   blank line above
