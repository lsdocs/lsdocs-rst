Title
=====

Before.

.. include:: files/part.txt

After the include `bad.

.. include:: files/missing.txt

.. include:: files/part.txt
   :start-after: START
   :end-before: END

.. include:: files/part.txt
   :literal:
   :number-lines: 3

.. include:: files/code.txt
   :code: python
   :number-lines:

.. include:: files/code.txt
   :literal:
   :tab-width: 2
   :class: c
   :name: lit

.. include:: files/part.txt
   :start-line: 2
   :end-line: -4
   :literal:

.. include:: files/part.txt
   :start-after: nowhere

.. include:: files/self.txt

   Quote with include:

   .. include:: files/inner.txt

   After `quote include.

Text after `x.
